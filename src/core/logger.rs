use std::io::Write;

pub(crate) fn init_logger() {
    let mut builder =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"));

    builder.format(|buf, record| {
        let level_style = buf.default_level_style(record.level());

        writeln!(
            buf,
            "{}[{}]{} {}",
            level_style.render(),
            record.level(),
            level_style.render_reset(),
            record.args()
        )
    });

    builder.init();
}
