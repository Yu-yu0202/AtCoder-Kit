use crate::APP_NAME;
use anyhow::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::LazyLock;

static TEMPLATE_DIR: LazyLock<PathBuf> = LazyLock::new(|| {
    let dir = confy::get_configuration_file_path(APP_NAME, None)
        .expect("Failed to get template directory.")
        .parent()
        .expect("Failed to get template directory.")
        .join("template");

    fs::create_dir_all(&dir).expect("Failed to create template directory.");

    dir
});

static ATCODER_LANGUAGES_JSON: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/languages-2025_2026.bin.zst"));

static ATCODER_LANGUAGES: LazyLock<Vec<AtCoderLanguage>> = LazyLock::new(|| {
    let decompressed =
        zstd::decode_all(ATCODER_LANGUAGES_JSON).expect("Failed to decompress languages.");
    postcard::from_bytes(&decompressed).expect("Failed to deserialize languages.")
});

fn select_language_id() -> Result<u16> {
    use dialoguer::{FuzzySelect, theme::ColorfulTheme};

    let items: Vec<&String> = ATCODER_LANGUAGES.iter().map(|l| &l.n).collect();

    log::info!("Select the language for AtCoder submission:");

    let selection = FuzzySelect::with_theme(&ColorfulTheme::default())
        .items(&items)
        .default(0)
        .interact_opt()
        .context("Failed to select language.")?;

    match selection {
        Some(index) => Ok(ATCODER_LANGUAGES[index].v.clone()),
        None => bail!("Language selection was cancelled."),
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct AtCoderLanguage {
    n: String,
    v: u16,
}

#[derive(Default, Serialize, Deserialize, Clone, Debug)]
pub struct TemplateData {
    pub template_path: PathBuf,
    pub name: String,
    pub config: TemplateConfig,
    pub is_default: bool,
}

#[derive(Default, Serialize, Deserialize, Clone, Debug)]
pub struct TemplateConfig {
    pub name: String,
    pub submit_file: PathBuf,
    pub language_id: u16,
    pub exec_command: Vec<String>,
    pub compile_command: Option<Vec<String>>,
    pub pre_submit: Option<Vec<String>>,
}

#[derive(Default, Serialize, Deserialize, Clone, Debug)]
pub struct Template {
    templates: HashMap<String, TemplateData>,
}

static CONFIG_NAME: &'static str = "template";

impl Template {
    pub fn load() -> Result<Self> {
        confy::load::<Template>(APP_NAME, CONFIG_NAME)
            .context("Failed to load template configuration.")
    }

    pub fn save(&self) -> Result<()> {
        confy::store(APP_NAME, CONFIG_NAME, self).context("Failed to save template configuration.")
    }

    pub fn register(&mut self, template: TemplateData) -> Result<()> {
        if template.is_default {
            if let Some(t) = self.templates.values_mut().find(|t| t.is_default) {
                t.is_default = false;
            }
        }

        self.templates.insert(template.name.clone(), template);

        self.save()?;

        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<&TemplateData> {
        self.templates.get(name)
    }

    pub fn get_default(&self) -> Option<&TemplateData> {
        self.templates.values().find(|t| t.is_default)
    }
}

impl TemplateData {
    pub fn new(template: &TemplateConfig, is_default: bool) -> Result<Self> {
        let template_path = TEMPLATE_DIR.join(&template.name);

        if template_path.exists() {
            return Err(anyhow!("Template with the same name already exists."));
        }

        fs::create_dir_all(
            &template_path
                .join(&template.submit_file)
                .parent()
                .context("Failed to create template directory.")?,
        )
        .context("Failed to create template directory.")?;

        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(template_path.join(&template.submit_file))?;

        Ok(Self {
            template_path,
            name: template.name.clone(),
            config: template.clone(),
            is_default,
        })
    }
}

pub fn new_template(
    name: &str,
    submit_file: &str,
    exec_command: &str,
    compile_command: Option<&str>,
    pre_submit: Option<&str>,
    is_default: bool,
) -> Result<PathBuf> {
    let exec_command =
        shell_words::split(&exec_command).context("Failed to parse exec command.")?;

    let compile_command = compile_command
        .map(|c| shell_words::split(&c))
        .transpose()
        .context("Failed to parse compile command.")?;

    let pre_submit = pre_submit
        .map(|c| shell_words::split(&c))
        .transpose()
        .context("Failed to parse pre-submit command.")?
        .filter(|v| !v.is_empty());

    let language_id = select_language_id()?;

    let config = TemplateConfig {
        name: name.to_string(),
        submit_file: PathBuf::from(submit_file),
        exec_command,
        compile_command,
        pre_submit,
        language_id,
    };

    let data = TemplateData::new(&config, is_default)?;
    let path = data.template_path.clone();

    fs::create_dir_all(&data.template_path).context("Failed to create template directory.")?;

    let mut store = Template::load()?;
    store.register(data)?;

    let config_file = fs::File::create(&path.join("template.json"))
        .context("Failed to create 'template.json'.")?;
    let writer = io::BufWriter::new(config_file);

    serde_json::to_writer_pretty(writer, &config)
        .context("Failed to write template configuration.")?;

    Ok(path)
}

pub fn from_file() -> Result<TemplateConfig> {
    let path = env::current_dir()
        .context("Failed to get current directory.")?
        .join("template.json");

    if !path.exists() {
        bail!("'template.json' does not exist.")
    }

    let file = fs::File::open(path)?;
    let reader = io::BufReader::new(file);

    let config = serde_json::from_reader(reader).context("Failed to read template.json.")?;

    Ok(config)
}
