use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct IPCCommand {
    /// Read and Reset are the two supported actions
    pub action: String,
}
