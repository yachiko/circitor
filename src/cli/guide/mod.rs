//! The guide (CLI-11; T11). One text, `guide.md`. `skills/circitor/SKILL.md` and `AGENTS.section.md`
//! are copies of it with a header each (OPS-12); a test keeps the three equal.

/// CLI-11, under 120 lines.
pub const GUIDE: &str = include_str!("guide.md");

#[cfg(test)]
mod tests;
