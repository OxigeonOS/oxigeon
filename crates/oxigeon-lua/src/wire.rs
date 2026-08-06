//! The bytes that cross between the server and a compute worker.
//!
//! Hand-rolled rather than serde, for one reason: [`LuaData`] already exists
//! precisely because the obvious serialization (`lua_to_json`) *cannot express a
//! Lua value* — it flattens `1` and `1.0`, cannot hold a byte string, and has
//! one composite type where Lua has a sequence and a map at once. Reaching for a
//! generic format here would either reintroduce those losses or need as much
//! bespoke code as this, with the encoding hidden in derives instead of written
//! down.
//!
//! Everything is little-endian. A frame is a `u32` length followed by that many
//! bytes; the body starts with a one-byte tag. Lengths are checked against
//! [`MAX_FRAME`] before anything is allocated, so a corrupt or hostile length
//! cannot make the reader reserve gigabytes.
//!
//! # The integer question
//!
//! With Lua 5.5 on one side and LuaJIT on the other, [`LuaData::Int`] versus
//! [`LuaData::Num`] stops being cosmetic: LuaJIT has one number type, so a job
//! returning `3` returns a float, while 5.5 would have returned an integer.
//! **The wire preserves whatever the sending VM produced and converts nothing.**
//! A LuaJIT worker's `3` therefore arrives in a 5.5 game VM as `3.0`, and
//! `tostring` renders it `3.0`. That is the honest answer — the alternative,
//! silently promoting integral floats to integers, would corrupt a job that
//! deliberately returned a whole-numbered float — and it is pinned by
//! `an_integral_float_stays_a_float` below.

use std::io::{self, Read, Write};

use crate::marshal::{Key, LuaData, Table};
use crate::settings::ComputeSettings;
use crate::vm::Ending;

/// Refuse any frame larger than this. Generous next to a real job's arguments,
/// and small enough that a bad length is an error rather than an allocation.
pub const MAX_FRAME: usize = 64 * 1024 * 1024;

/// Server to worker.
#[derive(Clone, Debug, PartialEq)]
pub enum ToWorker {
    /// Sent once, before anything else. The worker builds its VM from this.
    Hello {
        settings: ComputeSettings,
        mudlib: String,
        game: String,
        /// Distinguishes the PRNG sequences of workers started together.
        salt: u64,
    },
    Job {
        id: u64,
        module: String,
        func: String,
        args: LuaData,
        /// Milliseconds from the worker's receipt of this frame. 0 = none.
        ///
        /// A duration rather than an instant because the two processes have no
        /// shared clock; the server's own deadline still governs when the
        /// *caller* is answered, so a slow pipe can only make the worker give up
        /// early, never late.
        deadline_ms: u64,
    },
    /// Ask the running job to stop. Only has teeth when a budget is armed —
    /// without the hook, nothing inside the VM checks it. The server's fallback
    /// is to kill the process.
    Cancel { id: u64 },
}

/// Worker to server.
#[derive(Clone, Debug, PartialEq)]
pub enum ToServer {
    /// The VM is built and the worker is ready for jobs.
    Ready,
    /// The VM could not be built. The worker exits after sending this.
    Broken { error: String },
    Done {
        id: u64,
        ending: Ending,
        value: LuaData,
        error: Option<String>,
        logs: Vec<(String, String)>,
    },
}

// ─── frames ──────────────────────────────────────────────────────────────────

/// Write one length-prefixed frame and flush it.
///
/// Flushing matters: the worker blocks reading the next frame, so a job left in
/// a `BufWriter` is a deadlock that looks exactly like a hung job.
pub fn write_frame(w: &mut impl Write, body: &[u8]) -> io::Result<()> {
    if body.len() > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("frame of {} bytes is over the {MAX_FRAME} limit", body.len()),
        ));
    }
    w.write_all(&(body.len() as u32).to_le_bytes())?;
    w.write_all(body)?;
    w.flush()
}

/// Read one length-prefixed frame. `Ok(None)` at a clean end of stream.
pub fn read_frame(r: &mut impl Read) -> io::Result<Option<Vec<u8>>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_le_bytes(len) as usize;
    if len > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("frame claims {len} bytes, over the {MAX_FRAME} limit"),
        ));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    Ok(Some(body))
}

// ─── encoding primitives ─────────────────────────────────────────────────────

fn put_u8(out: &mut Vec<u8>, v: u8) {
    out.push(v);
}
fn put_u64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn put_i64(out: &mut Vec<u8>, v: i64) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn put_f64(out: &mut Vec<u8>, v: f64) {
    out.extend_from_slice(&v.to_bits().to_le_bytes());
}
fn put_bytes(out: &mut Vec<u8>, v: &[u8]) {
    put_u64(out, v.len() as u64);
    out.extend_from_slice(v);
}
fn put_str(out: &mut Vec<u8>, v: &str) {
    put_bytes(out, v.as_bytes());
}

/// A cursor over a frame body. Every read is bounds-checked, so a truncated or
/// malformed frame is a `DecodeError` rather than a panic in a worker's reader
/// thread.
struct Cur<'a> {
    b: &'a [u8],
    i: usize,
}

/// A frame that could not be decoded.
#[derive(Debug, PartialEq, Eq)]
pub struct DecodeError(pub String);

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "malformed compute frame: {}", self.0)
    }
}
impl std::error::Error for DecodeError {}

impl From<DecodeError> for io::Error {
    fn from(e: DecodeError) -> Self {
        io::Error::new(io::ErrorKind::InvalidData, e.to_string())
    }
}

type Dec<T> = Result<T, DecodeError>;

impl<'a> Cur<'a> {
    fn new(b: &'a [u8]) -> Self {
        Self { b, i: 0 }
    }
    fn take(&mut self, n: usize) -> Dec<&'a [u8]> {
        let end = self.i.checked_add(n).ok_or_else(|| DecodeError("length overflow".into()))?;
        if end > self.b.len() {
            return Err(DecodeError(format!(
                "wanted {n} bytes at offset {}, frame holds {}",
                self.i,
                self.b.len()
            )));
        }
        let out = &self.b[self.i..end];
        self.i = end;
        Ok(out)
    }
    fn u8(&mut self) -> Dec<u8> {
        Ok(self.take(1)?[0])
    }
    fn u64(&mut self) -> Dec<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn i64(&mut self) -> Dec<i64> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn f64(&mut self) -> Dec<f64> {
        Ok(f64::from_bits(u64::from_le_bytes(self.take(8)?.try_into().unwrap())))
    }
    fn bytes(&mut self) -> Dec<Vec<u8>> {
        let n = self.u64()? as usize;
        Ok(self.take(n)?.to_vec())
    }
    fn string(&mut self) -> Dec<String> {
        let b = self.bytes()?;
        String::from_utf8(b).map_err(|_| DecodeError("string is not utf-8".into()))
    }
    fn usize(&mut self) -> Dec<usize> {
        Ok(self.u64()? as usize)
    }
}

// ─── LuaData ─────────────────────────────────────────────────────────────────

const T_NIL: u8 = 0;
const T_BOOL: u8 = 1;
const T_INT: u8 = 2;
const T_NUM: u8 = 3;
const T_STR: u8 = 4;
const T_TABLE: u8 = 5;

fn put_data(out: &mut Vec<u8>, d: &LuaData) {
    match d {
        LuaData::Nil => put_u8(out, T_NIL),
        LuaData::Bool(b) => {
            put_u8(out, T_BOOL);
            put_u8(out, *b as u8);
        }
        LuaData::Int(i) => {
            put_u8(out, T_INT);
            put_i64(out, *i);
        }
        LuaData::Num(n) => {
            put_u8(out, T_NUM);
            put_f64(out, *n);
        }
        LuaData::Str(s) => {
            put_u8(out, T_STR);
            put_bytes(out, s);
        }
        LuaData::Table(t) => {
            put_u8(out, T_TABLE);
            put_u64(out, t.seq.len() as u64);
            for v in &t.seq {
                put_data(out, v);
            }
            put_u64(out, t.map.len() as u64);
            for (k, v) in &t.map {
                match k {
                    Key::Int(i) => {
                        put_u8(out, T_INT);
                        put_i64(out, *i);
                    }
                    Key::Str(s) => {
                        put_u8(out, T_STR);
                        put_bytes(out, s);
                    }
                }
                put_data(out, v);
            }
        }
    }
}

/// Nesting bound for decoding, mirroring `Limits::depth`'s default.
///
/// The encoder cannot emit a cycle — `marshal::from_lua` already refused one —
/// but a *decoder* reads whatever arrives, and recursion on hostile input is how
/// a reader thread gets a stack overflow instead of an error.
const MAX_DEPTH: usize = 256;

fn get_data(c: &mut Cur, depth: usize) -> Dec<LuaData> {
    if depth > MAX_DEPTH {
        return Err(DecodeError(format!("value nests deeper than {MAX_DEPTH}")));
    }
    Ok(match c.u8()? {
        T_NIL => LuaData::Nil,
        T_BOOL => LuaData::Bool(c.u8()? != 0),
        T_INT => LuaData::Int(c.i64()?),
        T_NUM => LuaData::Num(c.f64()?),
        T_STR => LuaData::Str(c.bytes()?),
        T_TABLE => {
            let mut t = Table::default();
            let n = c.usize()?;
            t.seq.reserve(n.min(4096));
            for _ in 0..n {
                t.seq.push(get_data(c, depth + 1)?);
            }
            let n = c.usize()?;
            for _ in 0..n {
                let key = match c.u8()? {
                    T_INT => Key::Int(c.i64()?),
                    T_STR => Key::Str(c.bytes()?),
                    other => return Err(DecodeError(format!("bad table key tag {other}"))),
                };
                t.map.insert(key, get_data(c, depth + 1)?);
            }
            LuaData::Table(t)
        }
        other => return Err(DecodeError(format!("bad value tag {other}"))),
    })
}

// ─── messages ────────────────────────────────────────────────────────────────

const M_HELLO: u8 = 1;
const M_JOB: u8 = 2;
const M_CANCEL: u8 = 3;

const M_READY: u8 = 0x81;
const M_BROKEN: u8 = 0x82;
const M_DONE: u8 = 0x83;

impl ToWorker {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        match self {
            Self::Hello { settings, mudlib, game, salt } => {
                put_u8(&mut out, M_HELLO);
                put_u64(&mut out, settings.instruction_limit);
                put_u64(&mut out, settings.memory_mb as u64);
                put_u64(&mut out, settings.max_arg_depth as u64);
                put_u64(&mut out, settings.max_arg_nodes as u64);
                put_str(&mut out, mudlib);
                put_str(&mut out, game);
                put_u64(&mut out, *salt);
            }
            Self::Job { id, module, func, args, deadline_ms } => {
                put_u8(&mut out, M_JOB);
                put_u64(&mut out, *id);
                put_str(&mut out, module);
                put_str(&mut out, func);
                put_data(&mut out, args);
                put_u64(&mut out, *deadline_ms);
            }
            Self::Cancel { id } => {
                put_u8(&mut out, M_CANCEL);
                put_u64(&mut out, *id);
            }
        }
        out
    }

    pub fn decode(body: &[u8]) -> Dec<Self> {
        let mut c = Cur::new(body);
        Ok(match c.u8()? {
            M_HELLO => Self::Hello {
                settings: ComputeSettings {
                    instruction_limit: c.u64()?,
                    memory_mb: c.usize()?,
                    max_arg_depth: c.usize()?,
                    max_arg_nodes: c.usize()?,
                },
                mudlib: c.string()?,
                game: c.string()?,
                salt: c.u64()?,
            },
            M_JOB => Self::Job {
                id: c.u64()?,
                module: c.string()?,
                func: c.string()?,
                args: get_data(&mut c, 0)?,
                deadline_ms: c.u64()?,
            },
            M_CANCEL => Self::Cancel { id: c.u64()? },
            other => return Err(DecodeError(format!("bad server message tag {other}"))),
        })
    }
}

/// The wire's own numbering for [`Ending`], fixed here rather than derived from
/// the enum's declaration order — reordering that enum must not silently change
/// what a running worker's frames mean.
fn ending_code(e: Ending) -> u8 {
    match e {
        Ending::Ok => 0,
        Ending::Error => 1,
        Ending::LoadError => 2,
        Ending::Timeout => 3,
        Ending::Cancelled => 4,
        Ending::Budget => 5,
        Ending::Refused => 6,
    }
}

fn ending_from(code: u8) -> Dec<Ending> {
    Ok(match code {
        0 => Ending::Ok,
        1 => Ending::Error,
        2 => Ending::LoadError,
        3 => Ending::Timeout,
        4 => Ending::Cancelled,
        5 => Ending::Budget,
        6 => Ending::Refused,
        other => return Err(DecodeError(format!("bad ending code {other}"))),
    })
}

impl ToServer {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        match self {
            Self::Ready => put_u8(&mut out, M_READY),
            Self::Broken { error } => {
                put_u8(&mut out, M_BROKEN);
                put_str(&mut out, error);
            }
            Self::Done { id, ending, value, error, logs } => {
                put_u8(&mut out, M_DONE);
                put_u64(&mut out, *id);
                put_u8(&mut out, ending_code(*ending));
                put_data(&mut out, value);
                match error {
                    Some(e) => {
                        put_u8(&mut out, 1);
                        put_str(&mut out, e);
                    }
                    None => put_u8(&mut out, 0),
                }
                put_u64(&mut out, logs.len() as u64);
                for (level, message) in logs {
                    put_str(&mut out, level);
                    put_str(&mut out, message);
                }
            }
        }
        out
    }

    pub fn decode(body: &[u8]) -> Dec<Self> {
        let mut c = Cur::new(body);
        Ok(match c.u8()? {
            M_READY => Self::Ready,
            M_BROKEN => Self::Broken { error: c.string()? },
            M_DONE => {
                let id = c.u64()?;
                let ending = ending_from(c.u8()?)?;
                let value = get_data(&mut c, 0)?;
                let error = match c.u8()? {
                    0 => None,
                    _ => Some(c.string()?),
                };
                let n = c.usize()?;
                let mut logs = Vec::with_capacity(n.min(1024));
                for _ in 0..n {
                    logs.push((c.string()?, c.string()?));
                }
                Self::Done { id, ending, value, error, logs }
            }
            other => return Err(DecodeError(format!("bad worker message tag {other}"))),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn round_trip_to_worker(m: ToWorker) {
        assert_eq!(ToWorker::decode(&m.encode()).unwrap(), m);
    }
    fn round_trip_to_server(m: ToServer) {
        assert_eq!(ToServer::decode(&m.encode()).unwrap(), m);
    }

    fn sample_table() -> LuaData {
        let mut map = BTreeMap::new();
        map.insert(Key::Str(b"name".to_vec()), LuaData::Str(b"rat".to_vec()));
        map.insert(Key::Int(99), LuaData::Bool(false));
        map.insert(
            Key::Str(b"nested".to_vec()),
            LuaData::Table(Table { seq: vec![LuaData::Num(1.5)], map: BTreeMap::new() }),
        );
        LuaData::Table(Table {
            seq: vec![LuaData::Int(1), LuaData::Str(vec![255, 0, 254]), LuaData::Nil],
            map,
        })
    }

    #[test]
    fn every_value_shape_survives_the_wire() {
        for v in [
            LuaData::Nil,
            LuaData::Bool(true),
            LuaData::Int(-7),
            LuaData::Num(0.25),
            LuaData::Str(b"hi".to_vec()),
            LuaData::Table(Table::default()),
            sample_table(),
        ] {
            round_trip_to_worker(ToWorker::Job {
                id: 1,
                module: "compute.x".into(),
                func: "f".into(),
                args: v.clone(),
                deadline_ms: 500,
            });
        }
    }

    /// The type-domain crossing the two runtimes make real. A LuaJIT worker has
    /// one number type, so an integral result arrives as a float — and it must
    /// stay one, because the alternative is guessing.
    #[test]
    fn an_integral_float_stays_a_float() {
        let body = ToServer::Done {
            id: 1,
            ending: Ending::Ok,
            value: LuaData::Num(3.0),
            error: None,
            logs: vec![],
        }
        .encode();
        let ToServer::Done { value, .. } = ToServer::decode(&body).unwrap() else {
            panic!("expected Done")
        };
        assert_eq!(value, LuaData::Num(3.0));
        assert_ne!(value, LuaData::Int(3), "a float must not arrive as an integer");
    }

    /// Non-UTF-8 Lua strings are the reason keys and values carry byte lengths
    /// rather than being encoded as text.
    #[test]
    fn a_non_utf8_string_survives() {
        let v = LuaData::Str(vec![0xff, 0xfe, 0x00, b'o', b'k']);
        round_trip_to_server(ToServer::Done {
            id: 2,
            ending: Ending::Ok,
            value: v,
            error: None,
            logs: vec![],
        });
    }

    #[test]
    fn every_message_shape_survives_the_wire() {
        round_trip_to_worker(ToWorker::Hello {
            settings: ComputeSettings {
                instruction_limit: 5,
                memory_mb: 64,
                max_arg_depth: 8,
                max_arg_nodes: 9,
            },
            mudlib: "C:/x/mudlib".into(),
            game: "C:/x/game".into(),
            salt: 42,
        });
        round_trip_to_worker(ToWorker::Cancel { id: 9 });
        round_trip_to_server(ToServer::Ready);
        round_trip_to_server(ToServer::Broken { error: "no lua".into() });
        for ending in [
            Ending::Ok,
            Ending::Error,
            Ending::LoadError,
            Ending::Timeout,
            Ending::Cancelled,
            Ending::Budget,
            Ending::Refused,
        ] {
            round_trip_to_server(ToServer::Done {
                id: 3,
                ending,
                value: sample_table(),
                error: Some("why".into()),
                logs: vec![("info".into(), "a".into()), ("warn".into(), "b".into())],
            });
        }
    }

    #[test]
    fn frames_round_trip_through_a_pipe() {
        let mut buf = Vec::new();
        write_frame(&mut buf, b"one").unwrap();
        write_frame(&mut buf, b"two").unwrap();
        let mut r = buf.as_slice();
        assert_eq!(read_frame(&mut r).unwrap().as_deref(), Some(&b"one"[..]));
        assert_eq!(read_frame(&mut r).unwrap().as_deref(), Some(&b"two"[..]));
        assert_eq!(read_frame(&mut r).unwrap(), None, "clean end of stream");
    }

    /// A worker that dies mid-write leaves a partial frame. The reader must say
    /// so rather than block or hand back half a value.
    #[test]
    fn a_truncated_frame_is_an_error_not_a_hang() {
        let mut buf = Vec::new();
        write_frame(&mut buf, b"abcdefgh").unwrap();
        buf.truncate(6);
        let mut r = buf.as_slice();
        assert!(read_frame(&mut r).is_err());
    }

    #[test]
    fn a_truncated_body_is_refused_rather_than_panicking() {
        let body = ToServer::Done {
            id: 1,
            ending: Ending::Ok,
            value: sample_table(),
            error: None,
            logs: vec![],
        }
        .encode();
        for cut in 1..body.len() {
            // Every prefix must decode to an error, never a panic.
            let _ = ToServer::decode(&body[..cut]);
        }
        assert!(ToServer::decode(&body[..body.len() - 1]).is_err());
    }

    #[test]
    fn an_unknown_tag_is_refused() {
        assert!(ToWorker::decode(&[200]).is_err());
        assert!(ToServer::decode(&[200]).is_err());
        assert!(ToServer::decode(&[]).is_err());
    }

    /// Deep nesting on the *decode* side is hostile input, not a Lua cycle, and
    /// must not recurse the reader thread's stack away.
    #[test]
    fn nesting_past_the_decode_limit_is_refused() {
        let mut body = vec![M_JOB];
        body.extend_from_slice(&1u64.to_le_bytes());
        for s in ["compute.x", "f"] {
            body.extend_from_slice(&(s.len() as u64).to_le_bytes());
            body.extend_from_slice(s.as_bytes());
        }
        // A table holding a table holding a table, far past MAX_DEPTH.
        for _ in 0..(MAX_DEPTH + 10) {
            body.push(T_TABLE);
            body.extend_from_slice(&1u64.to_le_bytes()); // one seq entry
        }
        body.push(T_NIL);
        assert!(ToWorker::decode(&body).is_err());
    }

    #[test]
    fn an_oversized_frame_is_refused_before_allocating() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&(MAX_FRAME as u32 + 1).to_le_bytes());
        let mut r = buf.as_slice();
        assert!(read_frame(&mut r).is_err());
        assert!(write_frame(&mut Vec::new(), &vec![0u8; MAX_FRAME + 1]).is_err());
    }
}
