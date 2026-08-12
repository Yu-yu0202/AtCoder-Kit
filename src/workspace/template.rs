use crate::APP_NAME;
use crate::workspace::command::CommandSpec;
use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;

static ATCODER_LANGUAGES_DATA: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/languages-2025_2026.bin.zst"));

static ATCODER_LANGUAGES: LazyLock<Vec<AtCoderLanguage>> = LazyLock::new(|| {
    let decompressed =
        zstd::decode_all(ATCODER_LANGUAGES_DATA).expect("Embedded language data is invalid.");
    postcard::from_bytes(&decompressed).expect("Embedded language data is invalid.")
});

#[derive(Serialize, Deserialize, Clone, Debug)]
struct AtCoderLanguage {
    n: String,
    v: u16,
}

#[derive(Default, Serialize, Deserialize, Clone, Debug, Eq, PartialEq)]
pub(crate) struct StoredTemplateConfig {
    pub(crate) name: String,
    pub(crate) submit_file: PathBuf,
    pub(crate) language_id: u16,
    pub(crate) exec_command: Vec<String>,
    pub(crate) compile_command: Option<Vec<String>>,
    pub(crate) pre_submit: Option<Vec<String>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TemplateConfig {
    pub(crate) name: String,
    pub(crate) submit_file: PathBuf,
    pub(crate) language_id: u16,
    pub(crate) exec_command: CommandSpec,
    pub(crate) compile_command: Option<CommandSpec>,
    pub(crate) pre_submit: Option<CommandSpec>,
}

impl TryFrom<StoredTemplateConfig> for TemplateConfig {
    type Error = anyhow::Error;

    fn try_from(stored: StoredTemplateConfig) -> Result<Self> {
        validate_relative_file_path(&stored.submit_file)?;
        let exec_command = CommandSpec::from_words(stored.exec_command)?;
        let compile_command = optional_command(stored.compile_command)?;
        let pre_submit = optional_command(stored.pre_submit)?;

        Ok(Self {
            name: stored.name,
            submit_file: stored.submit_file,
            language_id: stored.language_id,
            exec_command,
            compile_command,
            pre_submit,
        })
    }
}

fn optional_command(words: Option<Vec<String>>) -> Result<Option<CommandSpec>> {
    words
        .filter(|words| !words.is_empty())
        .map(CommandSpec::from_words)
        .transpose()
}

#[derive(Default, Serialize, Deserialize, Clone, Debug)]
pub(crate) struct TemplateData {
    pub(crate) template_path: PathBuf,
    pub(crate) name: String,
    pub(crate) config: StoredTemplateConfig,
    pub(crate) is_default: bool,
}

#[derive(Default, Serialize, Deserialize, Clone, Debug)]
pub(crate) struct TemplateRegistry {
    templates: HashMap<String, TemplateData>,
}

const CONFIG_NAME: &str = "template";

impl TemplateRegistry {
    pub(crate) fn load() -> Result<Self> {
        confy::load(APP_NAME, CONFIG_NAME).context("Failed to load template configuration.")
    }

    fn save(&self) -> Result<()> {
        confy::store(APP_NAME, CONFIG_NAME, self).context("Failed to save template configuration.")
    }

    pub(crate) fn select(&self, name: Option<&str>) -> Result<Option<&TemplateData>> {
        if let Some(name) = name {
            return self
                .templates
                .get(name)
                .map(Some)
                .context("Template not found.");
        }
        Ok(self.templates.values().find(|template| template.is_default))
    }

    fn with_registered(&self, template: TemplateData) -> Self {
        let mut updated = self.clone();
        if template.is_default {
            for existing in updated.templates.values_mut() {
                existing.is_default = false;
            }
        }
        updated.templates.insert(template.name.clone(), template);
        updated
    }
}

fn template_root() -> Result<PathBuf> {
    let config_path = confy::get_configuration_file_path(APP_NAME, None)
        .context("Failed to get template directory.")?;
    let parent = config_path
        .parent()
        .context("Failed to get template directory.")?;
    Ok(parent.join("template"))
}

fn validate_template_name(name: &str) -> Result<()> {
    if name.is_empty() || name.contains('/') || name.contains('\\') {
        bail!("Template name must be a single path component.");
    }
    let mut components = Path::new(name).components();
    if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
        bail!("Template name must be a single path component.");
    }
    Ok(())
}

pub(crate) fn validate_relative_file_path(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        bail!("Submit file must be a non-empty relative path.");
    }
    let portable = path.to_string_lossy();
    let bytes = portable.as_bytes();
    if portable.starts_with("\\\\")
        || (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
        || portable
            .split(['/', '\\'])
            .any(|component| component == "..")
    {
        bail!("Submit file must stay inside the problem directory.");
    }
    let mut saw_normal = false;
    for component in path.components() {
        match component {
            Component::Normal(_) => saw_normal = true,
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                bail!("Submit file must stay inside the problem directory.");
            }
        }
    }
    if !saw_normal {
        bail!("Submit file must be a non-empty relative path.");
    }
    Ok(())
}

fn select_language_id() -> Result<u16> {
    use dialoguer::{FuzzySelect, theme::ColorfulTheme};

    let items = ATCODER_LANGUAGES
        .iter()
        .map(|language| &language.n)
        .collect::<Vec<_>>();
    log::info!("Select the language for AtCoder submission:");
    let selection = FuzzySelect::with_theme(&ColorfulTheme::default())
        .items(&items)
        .default(0)
        .interact_opt()
        .context("Failed to select language.")?;
    selection
        .map(|index| ATCODER_LANGUAGES[index].v)
        .context("Language selection was cancelled.")
}

pub(crate) struct NewTemplate<'a> {
    pub(crate) name: &'a str,
    pub(crate) submit_file: &'a str,
    pub(crate) exec_command: &'a str,
    pub(crate) compile_command: Option<&'a str>,
    pub(crate) pre_submit: Option<&'a str>,
    pub(crate) default: bool,
}

pub(crate) fn create_template(request: NewTemplate<'_>) -> Result<PathBuf> {
    validate_template_name(request.name)?;
    let submit_file = PathBuf::from(request.submit_file);
    validate_relative_file_path(&submit_file)?;

    let stored = StoredTemplateConfig {
        name: request.name.to_string(),
        submit_file,
        language_id: select_language_id()?,
        exec_command: shell_words::split(request.exec_command)
            .context("Failed to parse exec command.")?,
        compile_command: request
            .compile_command
            .map(shell_words::split)
            .transpose()
            .context("Failed to parse compile command.")?
            .filter(|words| !words.is_empty()),
        pre_submit: request
            .pre_submit
            .map(shell_words::split)
            .transpose()
            .context("Failed to parse pre-submit command.")?
            .filter(|words| !words.is_empty()),
    };
    TemplateConfig::try_from(stored.clone())?;

    let root = template_root()?;
    fs::create_dir_all(&root).context("Failed to create template directory.")?;
    let final_path = root.join(request.name);
    if final_path.exists() {
        bail!("Template with the same name already exists.");
    }

    let registry = TemplateRegistry::load()?;
    let staging = tempfile::Builder::new()
        .prefix(".ackit-template-")
        .tempdir_in(&root)
        .context("Failed to create template staging directory.")?;
    let staging_path = staging.path();
    let source_path = staging_path.join(&stored.submit_file);
    fs::create_dir_all(
        source_path
            .parent()
            .context("Failed to create template source directory.")?,
    )
    .context("Failed to create template source directory.")?;
    fs::File::create(&source_path).context("Failed to create template source file.")?;

    let config_path = staging_path.join("template.json");
    let mut config_file = io::BufWriter::new(
        fs::File::create(&config_path).context("Failed to create template.json.")?,
    );
    serde_json::to_writer_pretty(&mut config_file, &stored)
        .context("Failed to write template.json.")?;
    config_file
        .flush()
        .context("Failed to flush template.json.")?;
    drop(config_file);

    let staging_path = staging.keep();
    if let Err(error) = fs::rename(&staging_path, &final_path) {
        let _ = fs::remove_dir_all(&staging_path);
        return Err(error).context("Failed to install template directory.");
    }

    let data = TemplateData {
        template_path: final_path.clone(),
        name: request.name.to_string(),
        config: stored,
        is_default: request.default,
    };
    let updated = registry.with_registered(data);
    if let Err(error) = updated.save() {
        if let Err(rollback_error) = fs::remove_dir_all(&final_path) {
            return Err(anyhow!(
                "{error:#}; also failed to roll back '{}': {rollback_error}",
                final_path.display()
            ));
        }
        return Err(error);
    }

    Ok(final_path)
}

pub(crate) fn load_template_config_from(path: &Path) -> Result<TemplateConfig> {
    let file =
        fs::File::open(path).with_context(|| format!("Failed to open '{}'.", path.display()))?;
    let stored: StoredTemplateConfig = serde_json::from_reader(io::BufReader::new(file))
        .with_context(|| format!("Failed to read '{}'.", path.display()))?;
    stored.try_into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_legacy_template_json_without_pre_submit() {
        let stored: StoredTemplateConfig = serde_json::from_str(include_str!(
            "../../tests/fixtures/json/template_legacy.json"
        ))
        .unwrap();
        assert_eq!(stored.pre_submit, None);
        let runtime = TemplateConfig::try_from(stored).unwrap();
        assert_eq!(runtime.exec_command.words(), ["python", "main.py"]);
    }

    #[test]
    fn normalizes_empty_optional_commands() {
        let stored = StoredTemplateConfig {
            name: "test".into(),
            submit_file: "main.cpp".into(),
            language_id: 1,
            exec_command: vec!["./a.out".into()],
            compile_command: Some(Vec::new()),
            pre_submit: Some(Vec::new()),
        };
        let runtime = TemplateConfig::try_from(stored).unwrap();
        assert!(runtime.compile_command.is_none());
        assert!(runtime.pre_submit.is_none());
    }

    #[test]
    fn rejects_empty_required_command_and_escaping_paths() {
        let stored = StoredTemplateConfig {
            name: "test".into(),
            submit_file: "main.cpp".into(),
            language_id: 1,
            exec_command: Vec::new(),
            compile_command: None,
            pre_submit: None,
        };
        assert!(TemplateConfig::try_from(stored).is_err());
        assert!(validate_relative_file_path(Path::new("../main.cpp")).is_err());
        assert!(validate_relative_file_path(Path::new("..\\main.cpp")).is_err());
        assert!(validate_relative_file_path(Path::new("/tmp/main.cpp")).is_err());
        assert!(validate_relative_file_path(Path::new("C:\\temp\\main.cpp")).is_err());
        assert!(validate_relative_file_path(Path::new("src/main.rs")).is_ok());
        assert!(validate_relative_file_path(Path::new("src\\main.rs")).is_ok());
        assert!(validate_relative_file_path(Path::new("./main.rs")).is_ok());
    }
}
