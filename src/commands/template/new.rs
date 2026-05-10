use crate::core::template::new_template;
use anyhow::*;
use log::*;
pub async fn new(
    name: &str,
    submit_file: &str,
    exec_command: &str,
    compile_command: Option<&str>,
    pre_submit: Option<&str>,
    is_default: bool,
) -> Result<()> {
    info!("Creating new template '{}'...", name);
    let path = new_template(
        name,
        submit_file,
        exec_command,
        compile_command,
        pre_submit,
        is_default,
    )?;
    info!("Template '{}' created.", name);
    info!("Template directory: {}", path.display());
    Ok(())
}
