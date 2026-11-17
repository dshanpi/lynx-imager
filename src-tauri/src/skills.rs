//! Read-only built-in Skills catalog shared by the isolated Linux and MCU MCP domains.
//!
//! Skills are documentation, not executable capabilities. This module never launches a
//! process, resolves a device, or performs a hardware action.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::{Component, Path},
    sync::OnceLock,
};

use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

pub const MAX_READ_BYTES: usize = 32 * 1024;
const DEFAULT_READ_BYTES: usize = 16 * 1024;
const DEFAULT_SEARCH_LIMIT: usize = 10;
const MAX_SEARCH_LIMIT: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SkillDomain {
    Linux,
    Mcu,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SkillSearchRequest {
    /// Natural-language capability or problem description. Maximum 256 characters.
    pub query: String,
    /// Optional normalized system filter such as freertos, linux, buildroot, uboot, tina or fes.
    pub system: Option<String>,
    /// Optional category or category prefix, for example rtos/packages/usb.
    pub category: Option<String>,
    /// Maximum returned skills, default 10 and capped at 50.
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SkillGetRequest {
    /// Exact skill name returned by the search tool.
    pub name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SkillReadRequest {
    /// Exact skill name returned by the search tool.
    pub name: String,
    /// SKILL.md by default, or a relative reference path returned by the get tool.
    pub relative_path: Option<String>,
    /// UTF-8 byte offset, normally the nextOffset returned by the previous read.
    pub offset: Option<usize>,
    /// Requested bytes, default 16384 and capped at 32768.
    pub max_bytes: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
struct SkillFrontmatter {
    name: String,
    version: String,
    domain: SkillDomain,
    systems: Vec<String>,
    category: String,
    #[serde(default)]
    chip_vendors: Vec<String>,
    #[serde(default)]
    required_tools: Vec<String>,
    description: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SkillMetadata {
    name: String,
    version: String,
    domain: SkillDomain,
    systems: Vec<String>,
    category: String,
    chip_vendors: Vec<String>,
    required_tools: Vec<String>,
    description: String,
    entry_file: &'static str,
    files: Vec<String>,
    sha256: String,
}

struct SkillRecord {
    metadata: SkillMetadata,
    files: HashMap<&'static str, &'static str>,
    file_hashes: BTreeMap<String, String>,
}

struct SkillSource {
    entry: &'static str,
    files: &'static [(&'static str, &'static str)],
}

struct SkillsCatalog {
    skills: Vec<SkillRecord>,
}

#[derive(Debug, Clone)]
pub struct NativeSkillDocument {
    pub name: String,
    pub content: String,
    pub sha256: String,
}

static CATALOG: OnceLock<Result<SkillsCatalog, String>> = OnceLock::new();

const SERIAL_FILES: &[(&str, &str)] = &[(
    "SKILL.md",
    include_str!("../resources/skills/linux/serial-diagnostics/SKILL.md"),
)];
const FES_FILES: &[(&str, &str)] = &[
    (
        "SKILL.md",
        include_str!("../resources/skills/linux/allwinner-fes/SKILL.md"),
    ),
    (
        "references/nand-boundary.md",
        include_str!("../resources/skills/linux/allwinner-fes/references/nand-boundary.md"),
    ),
];
const T113_FILES: &[(&str, &str)] = &[(
    "SKILL.md",
    include_str!("../resources/skills/linux/t113s3-mainline/SKILL.md"),
)];
const T527_FILES: &[(&str, &str)] = &[(
    "SKILL.md",
    include_str!("../resources/skills/linux/t527-sdk-emmc/SKILL.md"),
)];
const CHERRYUSB_FILES: &[(&str, &str)] = &[(
    "SKILL.md",
    include_str!("../resources/skills/mcu/freertos-cherryusb/SKILL.md"),
)];
const LINUX_INCIDENT_FILES: &[(&str, &str)] = &[
    (
        "SKILL.md",
        include_str!("../resources/skills/linux/linux-incident-triage/SKILL.md"),
    ),
    (
        "agents/openai.yaml",
        include_str!("../resources/skills/linux/linux-incident-triage/agents/openai.yaml"),
    ),
];
const MCU_INCIDENT_FILES: &[(&str, &str)] = &[
    (
        "SKILL.md",
        include_str!("../resources/skills/mcu/mcu-incident-triage/SKILL.md"),
    ),
    (
        "agents/openai.yaml",
        include_str!("../resources/skills/mcu/mcu-incident-triage/agents/openai.yaml"),
    ),
];

fn built_in_sources() -> [SkillSource; 7] {
    [
        SkillSource {
            entry: "SKILL.md",
            files: SERIAL_FILES,
        },
        SkillSource {
            entry: "SKILL.md",
            files: FES_FILES,
        },
        SkillSource {
            entry: "SKILL.md",
            files: T113_FILES,
        },
        SkillSource {
            entry: "SKILL.md",
            files: T527_FILES,
        },
        SkillSource {
            entry: "SKILL.md",
            files: CHERRYUSB_FILES,
        },
        SkillSource {
            entry: "SKILL.md",
            files: LINUX_INCIDENT_FILES,
        },
        SkillSource {
            entry: "SKILL.md",
            files: MCU_INCIDENT_FILES,
        },
    ]
}

fn sha256(content: &str) -> String {
    format!("{:x}", Sha256::digest(content.as_bytes()))
}

fn parse_frontmatter(content: &str) -> Result<SkillFrontmatter, String> {
    let content = content
        .strip_prefix("---\n")
        .ok_or_else(|| "SKILLS_FRONTMATTER_INVALID:missing opening delimiter".to_owned())?;
    let (yaml, _) = content
        .split_once("\n---\n")
        .ok_or_else(|| "SKILLS_FRONTMATTER_INVALID:missing closing delimiter".to_owned())?;
    serde_yaml::from_str(yaml).map_err(|error| format!("SKILLS_FRONTMATTER_INVALID:{error}"))
}

fn validate_skill_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn allowed_linux_tools() -> &'static [&'static str] {
    &[
        "lynx_workbench_status",
        "lynx_flash_capabilities",
        "lynx_allwinner_enter_fel",
        "lynx_scan_flash_devices",
        "lynx_start_flash",
        "lynx_workbench_task_status",
        "lynx_serial_open",
        "lynx_serial_read",
        "lynx_serial_close",
    ]
}

fn allowed_mcu_tools() -> &'static [&'static str] {
    &[
        "lynx_mcu_capabilities",
        "lynx_mcu_board_info",
        "lynx_mcu_uart_open",
        "lynx_mcu_uart_read",
        "lynx_mcu_uart_close",
    ]
}

fn validate_required_tool(domain: SkillDomain, tool: &str) -> Result<(), String> {
    let allowed = match domain {
        SkillDomain::Linux => allowed_linux_tools(),
        SkillDomain::Mcu => allowed_mcu_tools(),
    };
    if allowed.contains(&tool) {
        Ok(())
    } else {
        Err(format!("SKILLS_REQUIRED_TOOL_INVALID:{tool}"))
    }
}

fn validate_relative_path(path: &str) -> Result<(), String> {
    if path.is_empty() || path.len() > 240 || Path::new(path).is_absolute() {
        return Err("SKILLS_PATH_INVALID:path must be a bounded relative path".to_owned());
    }
    if !Path::new(path)
        .components()
        .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err("SKILLS_PATH_INVALID:path traversal is prohibited".to_owned());
    }
    let extension = Path::new(path)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !matches!(extension.as_str(), "md" | "txt" | "json" | "yaml" | "yml") {
        return Err("SKILLS_PATH_INVALID:binary and executable files are prohibited".to_owned());
    }
    Ok(())
}

impl SkillsCatalog {
    fn load() -> Result<Self, String> {
        let mut names = BTreeSet::new();
        let mut skills = Vec::new();
        for source in built_in_sources() {
            let files: HashMap<_, _> = source.files.iter().copied().collect();
            let entry = files
                .get(source.entry)
                .ok_or_else(|| "SKILLS_ENTRY_MISSING:SKILL.md".to_owned())?;
            let frontmatter = parse_frontmatter(entry)?;
            if !validate_skill_name(&frontmatter.name) || !names.insert(frontmatter.name.clone()) {
                return Err(format!("SKILLS_NAME_INVALID:{}", frontmatter.name));
            }
            if frontmatter.systems.is_empty() || frontmatter.category.trim().is_empty() {
                return Err(format!("SKILLS_METADATA_INVALID:{}", frontmatter.name));
            }
            for tool in &frontmatter.required_tools {
                validate_required_tool(frontmatter.domain, tool)?;
            }
            let mut file_hashes = BTreeMap::new();
            for (path, content) in source.files {
                validate_relative_path(path)?;
                file_hashes.insert((*path).to_owned(), sha256(content));
            }
            let metadata = SkillMetadata {
                name: frontmatter.name,
                version: frontmatter.version,
                domain: frontmatter.domain,
                systems: frontmatter
                    .systems
                    .into_iter()
                    .map(|system| normalize_system_name(&system))
                    .collect(),
                category: frontmatter.category.to_ascii_lowercase(),
                chip_vendors: frontmatter
                    .chip_vendors
                    .into_iter()
                    .map(|vendor| vendor.to_ascii_lowercase())
                    .collect(),
                required_tools: frontmatter.required_tools,
                description: frontmatter.description,
                entry_file: source.entry,
                files: file_hashes.keys().cloned().collect(),
                sha256: sha256(entry),
            };
            skills.push(SkillRecord {
                metadata,
                files,
                file_hashes,
            });
        }
        Ok(Self { skills })
    }

    fn skill(&self, domain: SkillDomain, name: &str) -> Result<&SkillRecord, String> {
        let normalized = name.trim().to_ascii_lowercase();
        self.skills
            .iter()
            .find(|skill| skill.metadata.domain == domain && skill.metadata.name == normalized)
            .ok_or_else(|| format!("SKILLS_NOT_FOUND:{normalized}"))
    }

    fn systems(&self, domain: SkillDomain) -> Vec<String> {
        self.skills
            .iter()
            .filter(|skill| skill.metadata.domain == domain)
            .flat_map(|skill| skill.metadata.systems.iter().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }
}

fn catalog() -> Result<&'static SkillsCatalog, String> {
    CATALOG
        .get_or_init(SkillsCatalog::load)
        .as_ref()
        .map_err(Clone::clone)
}

fn normalize_system_name(system: &str) -> String {
    let normalized = system.trim().to_ascii_lowercase().replace(['_', ' '], "-");
    match normalized.as_str() {
        "free-rtos" => "freertos".to_owned(),
        "u-boot" => "uboot".to_owned(),
        "allwinner-fes" => "fes".to_owned(),
        _ => normalized,
    }
}

fn query_tokens(query: &str) -> Vec<String> {
    query
        .to_ascii_lowercase()
        .split(|character: char| {
            !character.is_alphanumeric() && character != '-' && character != '_'
        })
        .filter(|token| token.len() >= 2)
        .map(ToOwned::to_owned)
        .collect()
}

fn score_skill(skill: &SkillMetadata, query: &str, tokens: &[String]) -> usize {
    let haystack = format!(
        "{} {} {} {} {}",
        skill.name,
        skill.description,
        skill.category,
        skill.systems.join(" "),
        skill.chip_vendors.join(" ")
    )
    .to_ascii_lowercase();
    let normalized_query = query.trim().to_ascii_lowercase();
    let mut score = usize::from(haystack.contains(&normalized_query)) * 20;
    for token in tokens {
        if skill.name.contains(token) {
            score += 8;
        }
        if skill.category.contains(token) || skill.systems.iter().any(|system| system == token) {
            score += 5;
        }
        if haystack.contains(token) {
            score += 2;
        }
    }
    score
}

pub fn search(
    domain: SkillDomain,
    vendor: Option<&str>,
    request: SkillSearchRequest,
) -> Result<String, String> {
    let query = request.query.trim();
    if query.is_empty() || query.chars().count() > 256 {
        return Err("SKILLS_QUERY_INVALID:query must contain 1..256 characters".to_owned());
    }
    let catalog = catalog()?;
    let systems = catalog.systems(domain);
    let system = request.system.as_deref().map(normalize_system_name);
    if let Some(system) = &system {
        if system != "common" && !systems.contains(system) {
            return Err(format!("SKILLS_SYSTEM_UNSUPPORTED:{system}"));
        }
    }
    let category = request
        .category
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase);
    let vendor = vendor
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase);
    let tokens = query_tokens(query);
    let mut matches = catalog
        .skills
        .iter()
        .filter(|skill| skill.metadata.domain == domain)
        .filter(|skill| {
            system.as_ref().is_none_or(|system| {
                skill.metadata.systems.contains(system)
                    || skill.metadata.systems.iter().any(|value| value == "common")
            })
        })
        .filter(|skill| {
            category
                .as_ref()
                .is_none_or(|category| skill.metadata.category.starts_with(category))
        })
        .filter(|skill| {
            skill.metadata.chip_vendors.is_empty()
                || vendor.as_ref().is_some_and(|vendor| {
                    skill
                        .metadata
                        .chip_vendors
                        .iter()
                        .any(|value| value == vendor)
                })
        })
        .filter_map(|skill| {
            let score = score_skill(&skill.metadata, query, &tokens);
            (score > 0).then_some((score, skill.metadata.clone()))
        })
        .collect::<Vec<_>>();
    matches.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.1.name.cmp(&right.1.name))
    });
    let total = matches.len();
    let limit = request
        .limit
        .unwrap_or(DEFAULT_SEARCH_LIMIT)
        .clamp(1, MAX_SEARCH_LIMIT);
    let results = matches
        .into_iter()
        .take(limit)
        .map(|(score, metadata)| json!({"score": score, "skill": metadata}))
        .collect::<Vec<_>>();
    Ok(json!({
        "query": query,
        "system": system,
        "category": category,
        "domain": domain,
        "matches": total,
        "results": results,
    })
    .to_string())
}

pub fn get(domain: SkillDomain, request: SkillGetRequest) -> Result<String, String> {
    let skill = catalog()?.skill(domain, &request.name)?;
    Ok(json!({
        "skill": skill.metadata,
        "fileHashes": skill.file_hashes,
        "next": {
            "tool": match domain { SkillDomain::Linux => "lynx_skills_read", SkillDomain::Mcu => "lynx_mcu_skills_read" },
            "relativePath": skill.metadata.entry_file,
        }
    })
    .to_string())
}

pub fn read(domain: SkillDomain, request: SkillReadRequest) -> Result<String, String> {
    let skill = catalog()?.skill(domain, &request.name)?;
    let path = request.relative_path.as_deref().unwrap_or("SKILL.md");
    validate_relative_path(path)?;
    let content = skill
        .files
        .get(path)
        .ok_or_else(|| format!("SKILLS_FILE_NOT_FOUND:{}:{path}", skill.metadata.name))?;
    let offset = request.offset.unwrap_or(0);
    let max_bytes = request
        .max_bytes
        .unwrap_or(DEFAULT_READ_BYTES)
        .clamp(1, MAX_READ_BYTES);
    let (chunk, end, eof) = bounded_utf8_chunk(content, offset, max_bytes)?;
    Ok(json!({
        "name": skill.metadata.name,
        "relativePath": path,
        "content": chunk,
        "offset": offset,
        "nextOffset": (!eof).then_some(end),
        "eof": eof,
        "sha256": skill.file_hashes.get(path),
    })
    .to_string())
}

fn bounded_utf8_chunk(
    content: &str,
    offset: usize,
    max_bytes: usize,
) -> Result<(&str, usize, bool), String> {
    if offset > content.len() || !content.is_char_boundary(offset) {
        return Err("SKILLS_OFFSET_INVALID:offset must be a returned UTF-8 boundary".to_owned());
    }
    let mut end = offset.saturating_add(max_bytes).min(content.len());
    while end > offset && !content.is_char_boundary(end) {
        end -= 1;
    }
    let eof = end == content.len();
    Ok((&content[offset..end], end, eof))
}

pub fn list_systems(domain: SkillDomain) -> Result<String, String> {
    Ok(json!({"domain": domain, "systems": catalog()?.systems(domain)}).to_string())
}

pub fn initialization_summary(domain: SkillDomain) -> Result<String, String> {
    let items = catalog()?
        .skills
        .iter()
        .filter(|skill| skill.metadata.domain == domain)
        .map(|skill| format!("{}: {}", skill.metadata.name, skill.metadata.description))
        .collect::<Vec<_>>()
        .join(" | ");
    Ok(items)
}

pub fn native_documents(domain: SkillDomain) -> Result<Vec<NativeSkillDocument>, String> {
    catalog()?
        .skills
        .iter()
        .filter(|skill| skill.metadata.domain == domain)
        .map(|skill| {
            let source = skill
                .files
                .get("SKILL.md")
                .ok_or_else(|| "SKILLS_ENTRY_MISSING:SKILL.md".to_owned())?;
            let (_, body) = source
                .strip_prefix("---\n")
                .and_then(|content| content.split_once("\n---\n"))
                .ok_or_else(|| "SKILLS_FRONTMATTER_INVALID:native render".to_owned())?;
            let content = format!(
                "---\nname: {}\ndescription: {}\n---\n{}",
                skill.metadata.name, skill.metadata.description, body
            );
            Ok(NativeSkillDocument {
                name: skill.metadata.name.clone(),
                sha256: sha256(&content),
                content,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn query_only_search_is_backward_compatible_and_domain_scoped() {
        let value: Value = serde_json::from_str(
            &search(
                SkillDomain::Mcu,
                None,
                SkillSearchRequest {
                    query: "UART CherryUSB CDC".to_owned(),
                    system: None,
                    category: None,
                    limit: None,
                },
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(value["matches"], 1);
        assert_eq!(value["results"][0]["skill"]["name"], "freertos-cherryusb");
        assert_eq!(value["results"][0]["skill"]["domain"], "mcu");
    }

    #[test]
    fn freertos_filter_and_alias_are_real_filters() {
        for system in ["freertos", "Free-RTOS", "free_rtos"] {
            let value: Value = serde_json::from_str(
                &search(
                    SkillDomain::Mcu,
                    None,
                    SkillSearchRequest {
                        query: "USB CDC".to_owned(),
                        system: Some(system.to_owned()),
                        category: Some("rtos/packages".to_owned()),
                        limit: Some(10),
                    },
                )
                .unwrap(),
            )
            .unwrap();
            assert_eq!(value["system"], "freertos");
            assert_eq!(value["matches"], 1);
        }
        assert!(search(
            SkillDomain::Mcu,
            None,
            SkillSearchRequest {
                query: "USB".to_owned(),
                system: Some("linux".to_owned()),
                category: None,
                limit: None,
            }
        )
        .unwrap_err()
        .starts_with("SKILLS_SYSTEM_UNSUPPORTED"));
    }

    #[test]
    fn path_traversal_binary_and_invalid_utf8_offsets_are_rejected() {
        for path in ["../SKILL.md", "/tmp/SKILL.md", "script.sh"] {
            assert!(read(
                SkillDomain::Linux,
                SkillReadRequest {
                    name: "allwinner-fes-flashing".to_owned(),
                    relative_path: Some(path.to_owned()),
                    offset: None,
                    max_bytes: None,
                }
            )
            .unwrap_err()
            .starts_with("SKILLS_PATH_INVALID"));
        }
        assert!(bounded_utf8_chunk("串口", 1, 4)
            .unwrap_err()
            .starts_with("SKILLS_OFFSET_INVALID"));
    }

    #[test]
    fn get_then_chunked_read_preserves_hash_and_reference_boundary() {
        let metadata: Value = serde_json::from_str(
            &get(
                SkillDomain::Linux,
                SkillGetRequest {
                    name: "allwinner-fes-flashing".to_owned(),
                },
            )
            .unwrap(),
        )
        .unwrap();
        assert!(metadata["fileHashes"]["references/nand-boundary.md"]
            .as_str()
            .is_some_and(|value| value.len() == 64));
        let chunk: Value = serde_json::from_str(
            &read(
                SkillDomain::Linux,
                SkillReadRequest {
                    name: "allwinner-fes-flashing".to_owned(),
                    relative_path: Some("references/nand-boundary.md".to_owned()),
                    offset: None,
                    max_bytes: Some(32),
                },
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(chunk["eof"], false);
        assert!(chunk["nextOffset"].as_u64().is_some());
    }

    #[test]
    fn required_tool_validation_rejects_cross_domain_capabilities() {
        assert!(validate_required_tool(SkillDomain::Linux, "lynx_mcu_uart_read").is_err());
        assert!(validate_required_tool(SkillDomain::Mcu, "lynx_adb_shell").is_err());
        assert!(catalog().is_ok());
    }

    #[test]
    fn incident_skills_are_advertised_and_native_frontmatter_is_portable() {
        let summary = initialization_summary(SkillDomain::Linux).unwrap();
        assert!(summary.contains("linux-incident-triage"));
        let document = native_documents(SkillDomain::Linux)
            .unwrap()
            .into_iter()
            .find(|item| item.name == "linux-incident-triage")
            .unwrap();
        let yaml = document
            .content
            .strip_prefix("---\n")
            .unwrap()
            .split_once("\n---\n")
            .unwrap()
            .0;
        let metadata: serde_yaml::Value = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(metadata.as_mapping().unwrap().len(), 2);
        assert!(metadata.get("name").is_some());
        assert!(metadata.get("description").is_some());
    }
}
