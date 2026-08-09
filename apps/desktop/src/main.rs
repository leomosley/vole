fn main() -> anyhow::Result<()> {
    let _config = vole::config::ConfigStore::new()?.load()?;
    Ok(())
}
