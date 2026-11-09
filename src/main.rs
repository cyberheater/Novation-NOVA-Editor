fn main() -> eframe::Result {
    let cli: Vec<std::path::PathBuf> = std::env::args_os().skip(1).map(std::path::PathBuf::from).collect();
    novaedit::run(cli)
}
