use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Default, Serialize, Deserialize, Clone, Debug, Eq, PartialEq)]
pub(crate) enum ContestType {
    #[serde(rename = "ABC")]
    Abc,
    #[serde(rename = "ARC")]
    Arc,
    #[serde(rename = "AGC")]
    Agc,
    #[serde(rename = "AHC")]
    Ahc,
    #[serde(rename = "AWC")]
    Awc,
    #[default]
    Other,
}

impl ContestType {
    pub(crate) fn from_contest_id(contest_id: &str) -> Self {
        if contest_id.starts_with("abc") {
            Self::Abc
        } else if contest_id.starts_with("arc") {
            Self::Arc
        } else if contest_id.starts_with("agc") {
            Self::Agc
        } else if contest_id.starts_with("ahc") {
            Self::Ahc
        } else if contest_id.starts_with("awc") {
            Self::Awc
        } else {
            Self::Other
        }
    }
}

#[derive(Default, Serialize, Deserialize, Clone, Debug, Eq, PartialEq)]
pub(crate) struct SampleCase {
    pub(crate) input: String,
    pub(crate) expected: String,
}

#[derive(Default, Serialize, Deserialize, Clone, Debug, Eq, PartialEq)]
pub(crate) struct Problem {
    pub(crate) id: String,
    pub(crate) contest_id: String,
    pub(crate) label: String,
    pub(crate) url: String,
    pub(crate) title: String,
    pub(crate) memory_limit_bytes: usize,
    pub(crate) time_limit_msecs: usize,
    pub(crate) sample_cases: Vec<SampleCase>,
}

#[derive(Default, Serialize, Deserialize, Clone, Debug, Eq, PartialEq)]
pub(crate) struct Contest {
    pub(crate) id: String,
    pub(crate) contest_type: ContestType,
    pub(crate) title: String,
    pub(crate) problems: HashMap<String, Problem>,
}
