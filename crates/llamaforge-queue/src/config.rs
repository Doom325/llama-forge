use crate::types::TaskType;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct AllowedPriorityRange {
    pub min: u32,
    pub max: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct RequestorClassPolicy {
    pub max_concurrent: u32,
    pub allowed_task_types: HashMap<TaskType, AllowedPriorityRange>,
    pub priority_bonus: i32,
    pub queue_timeout_secs: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StarvationConfig {
    pub check_interval_secs: u32,
    pub boost_scale: f64,
    pub boost_start_after_secs: u32,
    pub max_boost: u32,
}

impl Default for StarvationConfig {
    fn default() -> Self {
        Self {
            check_interval_secs: 5,
            boost_scale: 50.0,
            boost_start_after_secs: 30,
            max_boost: 400,
        }
    }
}
