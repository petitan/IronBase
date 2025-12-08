//! IronBase Backup CLI
//!
//! Command-line interface for backup and restore operations.

use clap::{Parser, Subcommand};
use ironbase_backup::{backup, chain, restore, verify};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "ironbase-backup")]
#[command(author, version, about = "IronBase incremental backup tool")]
#[command(propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a backup of the database
    #[command(alias = "b")]
    Backup {
        /// Path to the database file (.mlite)
        #[arg(long, short)]
        db: PathBuf,

        /// Output directory for backups
        #[arg(long, short)]
        output: PathBuf,

        /// Force a full backup even if incrementals are available
        #[arg(long, short)]
        full: bool,
    },

    /// Restore database from backups
    #[command(alias = "r")]
    Restore {
        /// Directory containing backup files
        #[arg(long, short)]
        dir: PathBuf,

        /// Output path for restored database
        #[arg(long, short)]
        output: PathBuf,

        /// Specific backup to restore to (defaults to latest)
        #[arg(long, short)]
        target: Option<String>,

        /// Database name (auto-detected if not specified)
        #[arg(long)]
        db_name: Option<String>,
    },

    /// List backups and show chain
    #[command(alias = "ls")]
    List {
        /// Directory containing backup files
        #[arg(long, short)]
        dir: PathBuf,
    },

    /// Verify backup integrity
    #[command(alias = "v")]
    Verify {
        /// Directory containing backup files
        #[arg(long, short)]
        dir: PathBuf,

        /// Database name (verifies all if not specified)
        #[arg(long)]
        db_name: Option<String>,
    },

    /// Show detailed info about a backup file
    #[command(alias = "i")]
    Info {
        /// Path to backup file (.ibak)
        #[arg(long, short)]
        file: PathBuf,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Backup { db, output, full } => cmd_backup(&db, &output, full),
        Commands::Restore {
            dir,
            output,
            target,
            db_name,
        } => cmd_restore(&dir, &output, target.as_deref(), db_name.as_deref()),
        Commands::List { dir } => cmd_list(&dir),
        Commands::Verify { dir, db_name } => cmd_verify(&dir, db_name.as_deref()),
        Commands::Info { file } => cmd_info(&file),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("\x1b[31mError:\x1b[0m {}", e);
            ExitCode::FAILURE
        }
    }
}

fn cmd_backup(db: &Path, output: &Path, full: bool) -> Result<(), Box<dyn std::error::Error>> {
    let result = backup::create_backup(db, output, full)?;
    println!();
    result.print_summary();
    Ok(())
}

fn cmd_restore(
    dir: &Path,
    output: &Path,
    target: Option<&str>,
    db_name: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let result = restore::restore(dir, output, target, db_name)?;
    println!();
    result.print_summary();
    Ok(())
}

fn cmd_list(dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let chains = chain::Chain::discover_all(dir)?;

    if chains.is_empty() {
        println!("No backups found in {}", dir.display());
        return Ok(());
    }

    println!("Backup chains in {}:\n", dir.display());

    for chain in chains {
        chain.print_summary();
        println!();
    }

    Ok(())
}

fn cmd_verify(dir: &Path, db_name: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let chains = if let Some(name) = db_name {
        vec![chain::Chain::discover(dir, name)?]
    } else {
        chain::Chain::discover_all(dir)?
    };

    if chains.is_empty() {
        println!("No backups found in {}", dir.display());
        return Ok(());
    }

    let mut all_valid = true;

    for chain in chains {
        println!("Database: {}\n", chain.db_name);
        let results = verify::verify_chain(&chain);
        verify::print_verify_results(&results);

        if results.iter().any(|r| !r.valid) {
            all_valid = false;
        }
        println!();
    }

    if !all_valid {
        return Err("Some backups failed verification".into());
    }

    Ok(())
}

fn cmd_info(file: &Path) -> Result<(), Box<dyn std::error::Error>> {
    verify::print_backup_info(file)?;
    Ok(())
}
