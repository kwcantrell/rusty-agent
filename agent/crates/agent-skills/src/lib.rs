//! Skills subsystem: discover, load-on-demand, author, and preload markdown
//! skill packages. Attaches to the agent core only through the `Tool` seam.

pub mod skill;
pub use skill::Skill;

// Uncommented as later tasks land:
pub mod guard;
pub mod presets;
pub mod registry;
pub mod tools;
pub use presets::{compose_system_prompt, SKILLS_AWARENESS};
pub use registry::{sanitize_slug, SkillRegistry};
pub use tools::{CreateSkill, ListSkills, ReadSkillFile, UseSkill};
