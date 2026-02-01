use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use glob::Pattern;
use sdec_tools::{decode_packet_json, format_decode_pretty, inspect_packet, InspectReport};

#[derive(Parser)]
#[command(
    name = "sdec-tools",
    version,
    about = "sdec inspection and decoding tools"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Inspect packet structure and sizes.
    Inspect {
        /// Path to the packet bytes.
        packet_path: PathBuf,
        /// Optional schema JSON for update summaries.
        #[arg(long)]
        schema: Option<PathBuf>,
        /// Optional glob filter when inspecting a directory.
        #[arg(long)]
        glob: Option<String>,
        /// Sort inspected packets.
        #[arg(long, value_enum)]
        sort: Option<InspectSort>,
        /// Limit the number of inspected packets (after sorting).
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Decode packet payloads into structured JSON.
    Decode {
        /// Path to the packet bytes.
        packet_file: PathBuf,
        /// Schema JSON describing the packet contents.
        #[arg(long)]
        schema: PathBuf,
        /// Output format.
        #[arg(long, value_enum, default_value_t = DecodeFormat::Json)]
        format: DecodeFormat,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum InspectSort {
    Size,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum DecodeFormat {
    Json,
    Pretty,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Inspect {
            packet_path,
            schema,
            glob,
            sort,
            limit,
        } => {
            let schema = schema
                .as_ref()
                .map(load_schema)
                .transpose()
                .context("load schema")?;
            if packet_path.is_dir() {
                let entries = collect_packet_entries(&packet_path, glob.as_deref())?;
                let mut entries = maybe_sort_entries(entries, sort);
                let limit = limit.or(sort.map(|InspectSort::Size| 10));
                if let Some(limit) = limit {
                    entries.truncate(limit);
                }
                for entry in entries {
                    let bytes = fs::read(&entry.path)
                        .with_context(|| format!("read packet {}", entry.path.display()))?;
                    let report = inspect_packet(
                        &bytes,
                        schema.as_ref(),
                        &wire::Limits::default(),
                        &codec::CodecLimits::default(),
                    )?;
                    println!("== {} ({} bytes) ==", entry.path.display(), entry.size);
                    print_inspect_report(&report);
                }
            } else {
                let bytes = fs::read(&packet_path)
                    .with_context(|| format!("read packet {}", packet_path.display()))?;
                let report = inspect_packet(
                    &bytes,
                    schema.as_ref(),
                    &wire::Limits::default(),
                    &codec::CodecLimits::default(),
                )?;
                print_inspect_report(&report);
            }
        }
        Command::Decode {
            packet_file,
            schema,
            format,
        } => {
            let bytes = fs::read(&packet_file)
                .with_context(|| format!("read packet {}", packet_file.display()))?;
            let schema = load_schema(&schema).context("load schema")?;
            let output = decode_packet_json(
                &bytes,
                &schema,
                &wire::Limits::default(),
                &codec::CodecLimits::default(),
            )?;
            match format {
                DecodeFormat::Json => {
                    let json = serde_json::to_string_pretty(&output).context("serialize json")?;
                    println!("{json}");
                }
                DecodeFormat::Pretty => {
                    println!("{}", format_decode_pretty(&output));
                }
            }
        }
    }
    Ok(())
}

fn load_schema(path: &PathBuf) -> Result<schema::Schema> {
    let contents =
        fs::read_to_string(path).with_context(|| format!("read schema {}", path.display()))?;
    let schema: schema::Schema = serde_json::from_str(&contents).context("parse schema json")?;
    schema
        .validate()
        .map_err(|err| anyhow::anyhow!("schema validation failed: {err:?}"))?;
    Ok(schema)
}

struct PacketEntry {
    path: PathBuf,
    size: u64,
}

fn collect_packet_entries(dir: &PathBuf, glob: Option<&str>) -> Result<Vec<PacketEntry>> {
    let mut entries = Vec::new();
    let pattern = match glob {
        Some(value) => Some(Pattern::new(value).context("invalid glob pattern")?),
        None => None,
    };

    for entry in fs::read_dir(dir).with_context(|| format!("read dir {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if let Some(pattern) = &pattern {
            let matches_path = pattern.matches_path(&path);
            let matches_name = path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| pattern.matches(name));
            if !matches_path && !matches_name {
                continue;
            }
        }
        let size = entry.metadata()?.len();
        entries.push(PacketEntry { path, size });
    }
    Ok(entries)
}

fn maybe_sort_entries(
    mut entries: Vec<PacketEntry>,
    sort: Option<InspectSort>,
) -> Vec<PacketEntry> {
    match sort {
        Some(InspectSort::Size) => {
            entries.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.path.cmp(&b.path)));
        }
        None => {}
    }
    entries
}

fn print_inspect_report(report: &InspectReport) {
    let header = report.header;
    println!(
        "version: {} flags: 0x{:04x} schema_hash: 0x{:016x}",
        header.version,
        header.flags.raw(),
        header.schema_hash
    );
    println!(
        "tick: {} baseline_tick: {} payload_len: {} bytes",
        header.tick, header.baseline_tick, header.payload_len
    );
    println!("sections:");
    for section in &report.sections {
        let tag = format!("{:?}", section.tag);
        let label = match section.tag {
            wire::SectionTag::EntityUpdateSparse | wire::SectionTag::EntityUpdateSparsePacked => {
                "entries"
            }
            _ => "entities",
        };
        let count = section
            .entity_count
            .map(|count| format!("{count} {label}"))
            .unwrap_or_else(|| "count n/a".to_string());
        println!("  {tag}: {count} ({} bytes)", section.byte_len);
    }
    let update_encoding = if report
        .sections
        .iter()
        .any(|section| section.tag == wire::SectionTag::EntityUpdateSparsePacked)
    {
        Some("sparse_packed")
    } else if report
        .sections
        .iter()
        .any(|section| section.tag == wire::SectionTag::EntityUpdateSparse)
    {
        Some("sparse_varint")
    } else if report
        .sections
        .iter()
        .any(|section| section.tag == wire::SectionTag::EntityUpdate)
    {
        Some("masked")
    } else {
        None
    };
    if let Some(encoding) = update_encoding {
        println!("update encoding: {encoding}");
    }
    if let Some(summary) = &report.update_summary {
        println!("update summary:");
        println!("  changed components: {}", summary.changed_components);
        println!("  changed fields: {}", summary.changed_fields);
        if !summary.by_component_fields.is_empty() {
            println!("  top components by changed fields:");
            for entry in &summary.by_component_fields {
                println!(
                    "    component {}: {} fields",
                    entry.component_id, entry.changed_fields
                );
            }
        }
    }
}
