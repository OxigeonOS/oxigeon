/// Telnet protocol byte constants per RFC 854

// IAC commands
pub const IAC: u8 = 255; // Interpret As Command
pub const SE: u8 = 240;  // Subnegotiation End
pub const NOP: u8 = 241; // No Operation
pub const DM: u8 = 242;  // Data Mark
pub const BRK: u8 = 243; // Break
pub const IP: u8 = 244;  // Interrupt Process
pub const AO: u8 = 245;  // Abort Output
pub const AYT: u8 = 246; // Are You There
pub const EC: u8 = 247;  // Erase Character
pub const EL: u8 = 248;  // Erase Line
pub const GA: u8 = 249;  // Go Ahead
pub const SB: u8 = 250;  // Subnegotiation Begin
pub const WILL: u8 = 251;
pub const WONT: u8 = 252;
pub const DO: u8 = 253;
pub const DONT: u8 = 254;

// Standard option codes
pub const OPT_ECHO: u8 = 1;       // Echo (RFC 857)
pub const OPT_SGA: u8 = 3;        // Suppress Go Ahead (RFC 858)
pub const OPT_TTYPE: u8 = 24;     // Terminal Type (RFC 1091)
pub const OPT_EOR: u8 = 25;       // End of Record (RFC 885)
pub const OPT_NAWS: u8 = 31;      // Negotiate About Window Size (RFC 1073)
pub const OPT_LINEMODE: u8 = 34;  // Linemode (RFC 1184)

// MUD-specific option codes
pub const OPT_MCCP2: u8 = 86;    // MUD Client Compression Protocol v2
pub const OPT_MCCP3: u8 = 87;    // MCCP v3 (client->server compression)
pub const OPT_GMCP: u8 = 201;    // Generic MUD Communication Protocol

// NVT line ending sequences
pub const CR: u8 = 13;
pub const LF: u8 = 10;
pub const NUL: u8 = 0;
