use crate::APP_NAME;
use anyhow::*;
use serde::{Deserialize, Serialize};

#[derive(Default, Serialize, Deserialize, Debug)]
pub struct Cookie {
    pub revel_session: String,
}

static CONFIG_NAME: &'static str = "cookie";

impl Cookie {
    pub fn load() -> Result<Self> {
        confy::load::<Cookie>(APP_NAME, CONFIG_NAME).context("Failed to load cookies.")
    }

    pub fn store(&self) -> Result<()> {
        confy::store(APP_NAME, CONFIG_NAME, self).context("Failed to store cookies.")
    }

    pub fn set_default() -> Result<()> {
        Cookie::default().store()
    }
}
