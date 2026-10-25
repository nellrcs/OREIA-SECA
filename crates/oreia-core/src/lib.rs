//! Núcleo do OREIA-SECA: tipos de mensagem e tarefa, fila com concorrência
//! controlada, broker de aprovação humana, classificador de intenção,
//! parser/registro de ações e armazenamento em disco.

pub mod actions;
pub mod approval;
pub mod intent;
pub mod message;
pub mod queue;
pub mod session;
pub mod store;
pub mod task;

pub use message::{ChatMessage, Incoming, Role, Source, WorkItem, WorkMode};
pub use task::{Phase, PhaseStatus, StepResolution, Task, TaskStatus};
