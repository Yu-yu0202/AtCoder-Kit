use crate::APP_NAME;
use anyhow::*;
use serde::{Deserialize, Serialize};

#[derive(Default, Serialize, Deserialize, Debug)]
pub(crate) struct Cookie {
    pub(crate) revel_session: String,
}

const CONFIG_NAME: &str = "cookie";

impl Cookie {
    pub(crate) fn load() -> Result<Self> {
        confy::load::<Cookie>(APP_NAME, CONFIG_NAME).context("Failed to load cookies.")
    }

    pub(crate) fn store(&self) -> Result<()> {
        confy::store(APP_NAME, CONFIG_NAME, self).context("Failed to store cookies.")
    }

    pub(crate) fn set_default() -> Result<()> {
        Cookie::default().store()
    }
}
