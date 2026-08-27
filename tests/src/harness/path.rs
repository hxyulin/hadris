pub fn join_path(parent: &str, name: &str) -> String {
    if parent == "/" {
        format!("/{name}")
    } else {
        format!("{parent}/{name}")
    }
}

pub fn split_parent(path: &str) -> Result<(&str, &str), String> {
    let index = path
        .rfind('/')
        .ok_or_else(|| format!("path is not absolute: {path}"))?;
    let parent = if index == 0 { "/" } else { &path[..index] };
    let name = &path[index + 1..];
    if name.is_empty() {
        Err(format!("path has no final component: {path}"))
    } else {
        Ok((parent, name))
    }
}

pub fn normalize_path(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{trimmed}")
    }
}

pub fn path_depth(path: &str) -> usize {
    path.split('/').filter(|part| !part.is_empty()).count()
}
