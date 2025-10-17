use clap::Parser;
use code_analyzer::analyzer::CodeAnalyzer;
use std::path::PathBuf;
use anyhow::Result;

#[derive(Parser)]
#[command(name = "analyze")]
#[command(about = "C code analyzer - equivalent to bash analyse_code.sh")]
struct Args {
    /// Path to analyze (file or directory)
    path: PathBuf,
    
    /// Rules file path
    #[arg(short, long, default_value = "rules.txt")]
    rules: PathBuf,
    
    /// Output format (text, json)
    #[arg(short, long, default_value = "text")]
    format: String,
    
    /// Verbose output
    #[arg(short, long)]
    verbose: bool,
    
    /// Exit with error code if issues found
    #[arg(long)]
    strict: bool,
    
    /// Only show errors (ignore warnings and info)
    #[arg(long)]
    errors_only: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    
    if !args.path.exists() {
        eprintln!("Error: Path '{}' does not exist", args.path.display());
        std::process::exit(1);
    }
    
    if args.verbose {
        eprintln!("Analyzing: {}", args.path.display());
        eprintln!("Rules file: {}", args.rules.display());
    }
    
    let analyzer = match CodeAnalyzer::new(args.rules) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("Failed to initialize analyzer: {}", e);
            std::process::exit(1);
        }
    };
    
    let results = match analyzer.analyze_path(&args.path, args.verbose) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Analysis failed: {}", e);
            std::process::exit(1);
        }
    };
    
    // Filter results if errors_only is set
    let filtered_results = if args.errors_only {
        results.into_iter().map(|mut result| {
            result.issues.retain(|issue| {
                matches!(issue.severity, code_analyzer::analyzer::Severity::Error)
            });
            result
        }).collect()
    } else {
        results
    };
    
    match args.format.as_str() {
        "json" => {
            let json = serde_json::to_string_pretty(&filtered_results)?;
            println!("{}", json);
        }
        _ => {
            analyzer.print_results(&filtered_results);
        }
    }
    
    // Exit with error code if strict mode and issues found
    if args.strict {
        let total_issues: usize = filtered_results.iter().map(|r| r.issues.len()).sum();
        if total_issues > 0 {
            std::process::exit(1);
        }
    }
    
    Ok(())
}