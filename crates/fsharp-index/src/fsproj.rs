//! Parser for F# project files (.fsproj) to extract compilation order.
//!
//! F# has a strict file compilation order defined in .fsproj files. Files earlier
//! in the order cannot reference symbols from files later in the order. This module
//! extracts that order to improve name resolution accuracy.

use quick_xml::events::Event;
use quick_xml::Reader;
use std::path::{Path, PathBuf};

/// Result of parsing a .fsproj file
#[derive(Debug, Clone, Default)]
pub struct FsprojInfo {
    /// Files in compilation order (first file is compiled first)
    pub compile_files: Vec<PathBuf>,
    /// Project references (paths to other .fsproj files)
    pub project_references: Vec<PathBuf>,
    /// NuGet package references
    pub package_references: Vec<PackageReference>,
    /// The directory containing the .fsproj file
    pub project_dir: PathBuf,
}

/// A NuGet package reference
#[derive(Debug, Clone)]
pub struct PackageReference {
    /// Package name (e.g., "Newtonsoft.Json")
    pub name: String,
    /// Package version (e.g., "13.0.1")
    pub version: String,
}

impl FsprojInfo {
    /// Get the compilation order index for a file (0 = first, higher = later)
    /// Returns None if the file is not in the project
    pub fn compilation_order(&self, file: &Path) -> Option<usize> {
        // Normalize the path for comparison
        let normalized = normalize_path(file);

        self.compile_files.iter().position(|f| {
            let project_file = normalize_path(f);
            paths_equal(&normalized, &project_file)
        })
    }

    /// Check if file A can reference file B based on compilation order.
    /// A can reference B only if B comes before A in compilation order.
    pub fn can_reference(&self, from_file: &Path, to_file: &Path) -> bool {
        match (
            self.compilation_order(from_file),
            self.compilation_order(to_file),
        ) {
            (Some(from_order), Some(to_order)) => to_order < from_order,
            // If either file is not in the project, allow the reference
            // (it might be an external file or test file)
            _ => true,
        }
    }

    /// Get all files that come before the given file in compilation order
    pub fn files_visible_from(&self, file: &Path) -> Vec<&PathBuf> {
        match self.compilation_order(file) {
            Some(order) => self.compile_files[..order].iter().collect(),
            None => self.compile_files.iter().collect(),
        }
    }
}

/// Parse a .fsproj file and extract compilation order
pub fn parse_fsproj(fsproj_path: &Path) -> Result<FsprojInfo, FsprojError> {
    let content = std::fs::read_to_string(fsproj_path).map_err(|e| FsprojError::IoError {
        path: fsproj_path.to_path_buf(),
        source: e,
    })?;

    let project_dir = fsproj_path.parent().unwrap_or(Path::new(".")).to_path_buf();

    parse_fsproj_content(&content, &project_dir)
}

/// Parse .fsproj XML content
pub fn parse_fsproj_content(content: &str, project_dir: &Path) -> Result<FsprojInfo, FsprojError> {
    let mut reader = Reader::from_str(content);
    reader.config_mut().trim_text(true);

    let mut info = FsprojInfo {
        project_dir: project_dir.to_path_buf(),
        ..Default::default()
    };

    let mut buf = Vec::new();
    let mut in_item_group = false;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                let name = e.name();
                let local_name = std::str::from_utf8(name.as_ref()).unwrap_or("");

                match local_name {
                    "ItemGroup" => {
                        in_item_group = true;
                    }
                    "Compile" if in_item_group => {
                        // Extract Include attribute
                        if let Some(include) = get_attribute(e, "Include") {
                            let file_path = project_dir.join(normalize_windows_path(&include));
                            info.compile_files.push(file_path);
                        }
                    }
                    "ProjectReference" if in_item_group => {
                        // Extract Include attribute for project references
                        if let Some(include) = get_attribute(e, "Include") {
                            let ref_path = project_dir.join(normalize_windows_path(&include));
                            info.project_references.push(ref_path);
                        }
                    }
                    "PackageReference" if in_item_group => {
                        // Extract Include and Version attributes for package references
                        if let Some(name) = get_attribute(e, "Include") {
                            let version = get_attribute(e, "Version").unwrap_or_default();
                            info.package_references
                                .push(PackageReference { name, version });
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::End(ref e)) => {
                let name = e.name();
                let local_name = std::str::from_utf8(name.as_ref()).unwrap_or("");
                if local_name == "ItemGroup" {
                    in_item_group = false;
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(FsprojError::ParseError {
                    message: format!("XML parse error: {}", e),
                });
            }
            _ => {}
        }
        buf.clear();
    }

    Ok(info)
}

/// Find .fsproj file(s) in a directory
pub fn find_fsproj_files(root: &Path) -> Vec<PathBuf> {
    let mut results = Vec::new();

    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(ext) = path.extension() {
                    if ext == "fsproj" {
                        results.push(path);
                    }
                }
            }
        }
    }

    // Also check subdirectories (but not recursively - just immediate children)
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && !is_hidden_dir(&path) {
                if let Ok(sub_entries) = std::fs::read_dir(&path) {
                    for sub_entry in sub_entries.flatten() {
                        let sub_path = sub_entry.path();
                        if sub_path.is_file() {
                            if let Some(ext) = sub_path.extension() {
                                if ext == "fsproj" {
                                    results.push(sub_path);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    results
}

/// Find the .fsproj file that contains a given F# source file
pub fn find_fsproj_for_file(file: &Path, search_root: &Path) -> Option<PathBuf> {
    // Start from the file's directory and walk up
    let mut current = file.parent()?;

    loop {
        // Check for .fsproj files in this directory
        if let Ok(entries) = std::fs::read_dir(current) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    if let Some(ext) = path.extension() {
                        if ext == "fsproj" {
                            // Check if this project contains our file
                            if let Ok(info) = parse_fsproj(&path) {
                                if info.compilation_order(file).is_some() {
                                    return Some(path);
                                }
                            }
                        }
                    }
                }
            }
        }

        // Move up to parent directory
        if current == search_root || current.parent().is_none() {
            break;
        }
        current = current.parent()?;
    }

    None
}

/// Errors that can occur when parsing .fsproj files
#[derive(Debug, thiserror::Error)]
pub enum FsprojError {
    #[error("Failed to read {path}: {source}")]
    IoError {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("Failed to parse .fsproj: {message}")]
    ParseError { message: String },
}

// Helper functions

fn get_attribute(element: &quick_xml::events::BytesStart, name: &str) -> Option<String> {
    for attr in element.attributes().flatten() {
        if std::str::from_utf8(attr.key.as_ref()).ok()? == name {
            return std::str::from_utf8(&attr.value).ok().map(|s| s.to_string());
        }
    }
    None
}

fn normalize_windows_path(path: &str) -> String {
    // Convert Windows-style paths to Unix-style
    path.replace('\\', "/")
}

fn normalize_path(path: &Path) -> PathBuf {
    // Canonicalize if possible, otherwise just clean up the path
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn paths_equal(a: &Path, b: &Path) -> bool {
    // Compare paths, handling different representations
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a_canon), Ok(b_canon)) => a_canon == b_canon,
        _ => {
            // Fallback to string comparison with normalized separators
            let a_str = a.to_string_lossy().replace('\\', "/");
            let b_str = b.to_string_lossy().replace('\\', "/");
            a_str == b_str || a_str.ends_with(&b_str) || b_str.ends_with(&a_str)
        }
    }
}

fn is_hidden_dir(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.starts_with('.'))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_fsproj() {
        let content = r#"
<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <TargetFramework>net8.0</TargetFramework>
  </PropertyGroup>
  <ItemGroup>
    <Compile Include="Types.fs" />
    <Compile Include="Utils.fs" />
    <Compile Include="Services.fs" />
    <Compile Include="Program.fs" />
  </ItemGroup>
</Project>
"#;

        let info = parse_fsproj_content(content, Path::new("/project")).unwrap();

        assert_eq!(info.compile_files.len(), 4);
        assert_eq!(info.compile_files[0], PathBuf::from("/project/Types.fs"));
        assert_eq!(info.compile_files[1], PathBuf::from("/project/Utils.fs"));
        assert_eq!(info.compile_files[2], PathBuf::from("/project/Services.fs"));
        assert_eq!(info.compile_files[3], PathBuf::from("/project/Program.fs"));
    }

    #[test]
    fn test_parse_fsproj_with_subdirs() {
        let content = r#"
<Project Sdk="Microsoft.NET.Sdk">
  <ItemGroup>
    <Compile Include="src\Domain\Types.fs" />
    <Compile Include="src\Application\Services.fs" />
    <Compile Include="Program.fs" />
  </ItemGroup>
</Project>
"#;

        let info = parse_fsproj_content(content, Path::new("/project")).unwrap();

        assert_eq!(info.compile_files.len(), 3);
        // Windows paths should be converted to Unix-style
        assert_eq!(
            info.compile_files[0],
            PathBuf::from("/project/src/Domain/Types.fs")
        );
    }

    #[test]
    fn test_compilation_order() {
        let content = r#"
<Project Sdk="Microsoft.NET.Sdk">
  <ItemGroup>
    <Compile Include="A.fs" />
    <Compile Include="B.fs" />
    <Compile Include="C.fs" />
  </ItemGroup>
</Project>
"#;

        let info = parse_fsproj_content(content, Path::new("/project")).unwrap();

        assert_eq!(info.compilation_order(Path::new("/project/A.fs")), Some(0));
        assert_eq!(info.compilation_order(Path::new("/project/B.fs")), Some(1));
        assert_eq!(info.compilation_order(Path::new("/project/C.fs")), Some(2));
        assert_eq!(info.compilation_order(Path::new("/project/D.fs")), None);
    }

    #[test]
    fn test_can_reference() {
        let content = r#"
<Project Sdk="Microsoft.NET.Sdk">
  <ItemGroup>
    <Compile Include="A.fs" />
    <Compile Include="B.fs" />
    <Compile Include="C.fs" />
  </ItemGroup>
</Project>
"#;

        let info = parse_fsproj_content(content, Path::new("/project")).unwrap();

        // C can reference A and B
        assert!(info.can_reference(Path::new("/project/C.fs"), Path::new("/project/A.fs")));
        assert!(info.can_reference(Path::new("/project/C.fs"), Path::new("/project/B.fs")));

        // B can reference A
        assert!(info.can_reference(Path::new("/project/B.fs"), Path::new("/project/A.fs")));

        // A cannot reference B or C
        assert!(!info.can_reference(Path::new("/project/A.fs"), Path::new("/project/B.fs")));
        assert!(!info.can_reference(Path::new("/project/A.fs"), Path::new("/project/C.fs")));

        // B cannot reference C
        assert!(!info.can_reference(Path::new("/project/B.fs"), Path::new("/project/C.fs")));
    }

    #[test]
    fn test_files_visible_from() {
        let content = r#"
<Project Sdk="Microsoft.NET.Sdk">
  <ItemGroup>
    <Compile Include="A.fs" />
    <Compile Include="B.fs" />
    <Compile Include="C.fs" />
  </ItemGroup>
</Project>
"#;

        let info = parse_fsproj_content(content, Path::new("/project")).unwrap();

        // From A, nothing is visible
        let visible_from_a = info.files_visible_from(Path::new("/project/A.fs"));
        assert!(visible_from_a.is_empty());

        // From B, A is visible
        let visible_from_b = info.files_visible_from(Path::new("/project/B.fs"));
        assert_eq!(visible_from_b.len(), 1);

        // From C, A and B are visible
        let visible_from_c = info.files_visible_from(Path::new("/project/C.fs"));
        assert_eq!(visible_from_c.len(), 2);
    }

    #[test]
    fn test_parse_project_references() {
        let content = r#"
<Project Sdk="Microsoft.NET.Sdk">
  <ItemGroup>
    <Compile Include="Program.fs" />
  </ItemGroup>
  <ItemGroup>
    <ProjectReference Include="..\Shared\Shared.fsproj" />
    <ProjectReference Include="..\Domain\Domain.fsproj" />
  </ItemGroup>
</Project>
"#;

        let info = parse_fsproj_content(content, Path::new("/project/App")).unwrap();

        assert_eq!(info.project_references.len(), 2);
        assert_eq!(
            info.project_references[0],
            PathBuf::from("/project/App/../Shared/Shared.fsproj")
        );
    }

    #[test]
    fn test_empty_fsproj() {
        let content = r#"
<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <TargetFramework>net8.0</TargetFramework>
  </PropertyGroup>
</Project>
"#;

        let info = parse_fsproj_content(content, Path::new("/project")).unwrap();

        assert!(info.compile_files.is_empty());
        assert!(info.project_references.is_empty());
    }
}
