use std::{
    collections::HashSet, env, error::Error, fs, io, iter::once, os::unix::process::CommandExt,
    path::PathBuf, process::Command,
};

use freedesktop_entry_parser::Entry;

const XDG_TERMINALS: &str = "xdg-terminals";
const XDG_TERMINALS_LIST: &str = "xdg-terminals.list";

fn desktops() -> Result<Vec<String>, env::VarError> {
    let xdg_current_desktop = env::var("XDG_CURRENT_DESKTOP")?;
    let ids = xdg_current_desktop
        // .to_ascii_lowercase()
        .split(':')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_owned())
        .collect();
    Ok(ids)
}

fn config_file_names(desktops: &[String]) -> Vec<String> {
    desktops
        .iter()
        .map(|desktop| format!("{}-{}", desktop, XDG_TERMINALS_LIST))
        .chain(once(XDG_TERMINALS_LIST.to_owned()))
        .collect()
}

fn config_paths(config_file_names: &[String]) -> Result<Vec<PathBuf>, xdg::BaseDirectoriesError> {
    let dirs = xdg::BaseDirectories::new()?;
    let config_dirs = once(dirs.get_config_home()).chain(dirs.get_config_dirs());
    let config_paths = config_dirs
        .flat_map(|dir| config_file_names.iter().map(move |path| dir.join(path)))
        .filter(|path| path.try_exists().unwrap_or(false));
    Ok(config_paths.collect())
}

fn configured_entries(desktops: &[String]) -> io::Result<Vec<PathBuf>> {
    let config_file_names = config_file_names(desktops);
    let config_paths = config_paths(&config_file_names)?;
    let configs = config_paths
        .iter()
        .map(fs::read_to_string)
        .collect::<io::Result<Vec<_>>>()?;
    let paths = configs
        .iter()
        .flat_map(|text: &String| text.lines().map(PathBuf::from))
        // TODO: ignore comments, blank lines
        .collect();
    Ok(paths)
}

fn present_entries(dirs: &xdg::BaseDirectories) -> io::Result<Vec<PathBuf>> {
    let dirs = once(dirs.get_data_home())
        .chain(dirs.get_data_dirs())
        .filter(|path| path.try_exists().unwrap_or(false));
    let dirs = dirs.map(fs::read_dir).collect::<io::Result<Vec<_>>>()?;
    let dirs = dirs.into_iter().flatten().collect::<io::Result<Vec<_>>>()?;
    let paths = dirs
        .iter()
        .map(|dir| PathBuf::from(dir.file_name()))
        .collect();
    Ok(paths)
}

fn entry(path: &PathBuf, desktops: &[String]) -> Option<Entry> {
    let entry = Entry::parse_file(path).ok()?;
    let section = entry.section("Desktop Entry");
    if section.attr("Hidden") == Some("true") {
        return None;
    }
    if let Some(not_show_in) = section.attr("NotShowIn") {
        if not_show_in
            .split_terminator(';')
            .any(|item| desktops.iter().any(|desktop| desktop == item))
        {
            return None;
        }
    }
    if let Some(only_show_in) = section.attr("OnlyShowIn") {
        if !only_show_in
            .split_terminator(';')
            .any(|item| desktops.iter().any(|desktop| desktop == item))
        {
            return None;
        }
    }
    if let Some(try_exec) = section.attr("TryExec") {
        if which::which(try_exec).is_err() {
            return None;
        }
    }
    Some(entry)
}

fn run(entry: &Entry, args: &[String]) -> Option<io::Error> {
    let section = entry.section("Desktop Entry");
    let mut cmd = Command::new("sh");
    cmd.arg("-c");
    let exec = section.attr("Exec").expect("attribute `Exec` is required");
    let exec_arg = section
        .attr("X-ExecArg")
        .or_else(|| section.attr("ExecArg"))
        .unwrap_or("-e");
    if args.is_empty() {
        cmd.arg(exec);
    } else {
        let mut exec = vec![exec.to_owned(), exec_arg.to_owned()];
        exec.extend_from_slice(args);
        cmd.arg(exec.join(" "));
    }
    Some(cmd.exec())
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let args: Vec<String> = env::args().skip(1).collect();
    let desktops = desktops()?;
    let dirs = xdg::BaseDirectories::with_prefix(XDG_TERMINALS)?;
    let mut handle = |entry_path: &PathBuf| {
        if !seen.insert(entry_path.clone()) {
            return;
        }
        dirs.find_data_file(entry_path)
            .and_then(|path| entry(&path, &desktops))
            .and_then(|entry| run(&entry, &args));
    };
    configured_entries(&desktops)?.iter().for_each(&mut handle);
    present_entries(&dirs)?.iter().for_each(&mut handle);
    Ok(())
}
