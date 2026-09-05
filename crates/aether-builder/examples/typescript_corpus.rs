use aether_builder::{GraphBuilder, Lang};
use aether_graph::SemanticGraph;
use std::path::{Path, PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let root = PathBuf::from(args.next().ok_or("usage: typescript_corpus ROOT OUTPUT")?);
    let output = PathBuf::from(args.next().ok_or("usage: typescript_corpus ROOT OUTPUT")?);
    if args.next().is_some() {
        return Err("usage: typescript_corpus ROOT OUTPUT".into());
    }
    let root = root.canonicalize()?;
    let mut paths = Vec::new();
    collect_paths(&root, &root, &mut paths)?;
    paths.sort();
    let sources = paths
        .iter()
        .map(|relative| {
            let source = std::fs::read_to_string(root.join(relative))?;
            Ok((relative.to_string_lossy().replace('\\', "/"), source))
        })
        .collect::<Result<Vec<_>, std::io::Error>>()?;

    let mut graph = SemanticGraph::new();
    let mut builder = GraphBuilder::new();
    builder.load_files(
        &mut graph,
        sources
            .iter()
            .map(|(relative, source)| (relative.as_str(), source.as_str())),
    );
    graph.save(output)?;
    println!(
        "{{\"schema_version\":1,\"source_files\":{},\"nodes\":{},\"edges\":{}}}",
        sources.len(),
        graph.node_count(),
        graph.edge_count()
    );
    Ok(())
}

fn collect_paths(root: &Path, directory: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    let mut entries = std::fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            return Err(std::io::Error::other(format!(
                "source tree contains a symlink: {}",
                entry.path().display()
            )));
        }
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .map_err(std::io::Error::other)?
            .to_path_buf();
        if excluded(&relative) {
            continue;
        }
        if file_type.is_dir() {
            collect_paths(root, &path, out)?;
        } else if file_type.is_file()
            && Lang::from_path(&relative.to_string_lossy()).is_some_and(Lang::is_typescript)
        {
            out.push(relative);
        }
    }
    Ok(())
}

fn excluded(relative: &Path) -> bool {
    relative.components().any(|component| {
        let component = component.as_os_str().to_string_lossy();
        component.starts_with('.')
            || matches!(
                component.as_ref(),
                "node_modules" | "target" | "__pycache__"
            )
    })
}
