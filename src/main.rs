use clap::{CommandFactory, Parser};
use git_workspace::commands::{
    add_provider_to_config, archive, completion, execute_cmd, fetch, list, lock,
    pull_all_repositories, update,
};
use git_workspace::config::ProviderSource;
use git_workspace::utils::{ensure_workspace_dir_exists, expand_workspace_path};
use std::path::PathBuf;
use log::{debug, info};

#[derive(clap::Parser)]
#[command(name = "git-workspace", author, about, version)]
struct Args {
    #[arg(short = 'w', long = "workspace", env = "GIT_WORKSPACE")]
    workspace: PathBuf,
    #[arg(short = 'v', long = "verbose", action = clap::ArgAction::Count, help = "Increase logging verbosity (-v for info, -vv for debug)")]
    verbose: u8,
    #[command(subcommand)]
    command: Command,
}

#[derive(clap::Parser, Debug)]
enum Command {
    /// Update the workspace, removing and adding any repositories as needed.
    Update {
        #[arg(short = 't', long = "threads", default_value = "8")]
        threads: usize,
    },
    /// Update the workspace without updating the lockfile (same as update but skips lock)
    OnlyUpdate {
        #[arg(short = 't', long = "threads", default_value = "8")]
        threads: usize,
    },
    /// Fetch new commits for all repositories in the workspace
    Fetch {
        #[arg(short = 't', long = "threads", default_value = "8")]
        threads: usize,
    },
    /// Fetch all repositories from configured providers and write the lockfile
    Lock {},
    /// Pull new commits on the primary branch for all repositories in the workspace
    SwitchAndPull {
        #[arg(short = 't', long = "threads", default_value = "8")]
        threads: usize,
    },
    /// List all repositories in the workspace
    ///
    /// This command will output the names of all known repositories in the workspace.
    /// Passing --full will output absolute paths.
    List {
        #[arg(long = "full")]
        full: bool,
    },
    /// Archive repositories that don't exist in the workspace anymore.
    Archive {
        /// Disable confirmation prompt
        #[arg(long = "force")]
        force: bool,
    },
    /// Run a git command in all repositories
    ///
    /// This command executes the "command" in all git workspace repositories.
    /// The program will receive the given "args", and have it's working directory
    /// set to the repository directory.
    Run {
        #[arg(short = 't', long = "threads", default_value = "8")]
        threads: usize,
        #[arg(required = true)]
        command: String,
        args: Vec<String>,
    },
    /// Add a provider to the configuration
    Add {
        #[arg(long = "file", default_value = "workspace.toml")]
        file: PathBuf,
        #[command(subcommand)]
        command: ProviderSource,
    },
    /// Generate shell completions
    Completion {
        /// The shell to generate the completion script for
        shell: clap_complete::Shell,
    },
}

fn main() -> anyhow::Result<()> {
    // Parse our arguments to Args using clap.
    let args = Args::parse();
    
    // Initialize logging based on verbosity level
    let log_level = match args.verbose {
        0 => "warn",
        1 => "info", 
        2 => "debug",
        _ => "trace",
    };
    
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or(log_level)
    ).init();
    
    debug!("Starting git-workspace with log level: {}", log_level);
    debug!("Workspace path: {:?}", args.workspace);
    
    handle_main(args)
}

fn handle_main(args: Args) -> anyhow::Result<()> {
    let workspace_path = expand_workspace_path(&args.workspace)?;
    let workspace_path = ensure_workspace_dir_exists(&workspace_path)?;
    
    info!("Using workspace directory: {}", workspace_path.display());
    debug!("Command: {:?}", args.command);

    // Run our sub command. Pretty self-explanatory.
    match args.command {
        Command::List { full } => list(&workspace_path, full)?,
        Command::Update { threads } => {
            lock(&workspace_path)?;
            update(&workspace_path, threads)?
        }
        Command::OnlyUpdate { threads } => {
            // Skip lock() call, only run update()
            update(&workspace_path, threads)?
        }
        Command::Lock {} => {
            lock(&workspace_path)?;
        }
        Command::Archive { force } => archive(&workspace_path, force)?,
        Command::Fetch { threads } => fetch(&workspace_path, threads)?,
        Command::Add { file, command } => add_provider_to_config(&workspace_path, command, &file)?,
        Command::Run {
            threads,
            command,
            args,
        } => execute_cmd(&workspace_path, threads, command, args)?,
        Command::SwitchAndPull { threads } => pull_all_repositories(&workspace_path, threads)?,
        Command::Completion { shell } => completion(shell, &mut Args::command())?,
    };
    Ok(())
}
