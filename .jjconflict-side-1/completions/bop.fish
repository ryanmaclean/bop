# Print an optspec for argparse to handle cmd's options that are independent of any subcommand.
function __fish_bop_global_optspecs
	string join \n cards-dir= h/help
end

function __fish_bop_needs_command
	# Figure out if the current invocation already has a command.
	set -l cmd (commandline -opc)
	set -e cmd[1]
	argparse -s (__fish_bop_global_optspecs) -- $cmd 2>/dev/null
	or return
	if set -q argv[1]
		# Also print the command, so this can be used to figure out what it is.
		echo $argv[1]
		return 1
	end
	return 0
end

function __fish_bop_using_subcommand
	set -l cmd (__fish_bop_needs_command)
	test -z "$cmd"
	and return 1
	contains -- $cmd[1] $argv
end

complete -c bop -n "__fish_bop_needs_command" -l cards-dir -r
complete -c bop -n "__fish_bop_needs_command" -s h -l help -d 'Print help'
complete -c bop -n "__fish_bop_needs_command" -f -a "init"
complete -c bop -n "__fish_bop_needs_command" -f -a "new"
complete -c bop -n "__fish_bop_needs_command" -f -a "create" -d 'Create a new job draft from a natural-language description'
complete -c bop -n "__fish_bop_needs_command" -f -a "status"
complete -c bop -n "__fish_bop_needs_command" -f -a "validate"
complete -c bop -n "__fish_bop_needs_command" -f -a "dispatcher"
complete -c bop -n "__fish_bop_needs_command" -f -a "merge-gate"
complete -c bop -n "__fish_bop_needs_command" -f -a "retry" -d 'Move a card back to pending/ so the dispatcher picks it up again'
complete -c bop -n "__fish_bop_needs_command" -f -a "kill" -d 'Send SIGTERM to the running agent and mark the card as failed'
complete -c bop -n "__fish_bop_needs_command" -f -a "approve" -d 'Approve a card that has decision_required set, unblocking it for dispatch'
complete -c bop -n "__fish_bop_needs_command" -f -a "logs" -d 'Stream stdout and stderr logs for a card'
complete -c bop -n "__fish_bop_needs_command" -f -a "inspect" -d 'Show meta, spec, and a log summary for a card'
complete -c bop -n "__fish_bop_needs_command" -f -a "memory" -d 'Manage per-template persistent memory'
complete -c bop -n "__fish_bop_needs_command" -f -a "serve" -d 'Start the REST API server for CI/CD integration'
complete -c bop -n "__fish_bop_needs_command" -f -a "worktree" -d 'Manage git worktrees associated with job cards'
complete -c bop -n "__fish_bop_needs_command" -f -a "providers" -d 'Manage AI providers (list, add, remove, status)'
complete -c bop -n "__fish_bop_needs_command" -f -a "config" -d 'Read and write global/project config settings'
complete -c bop -n "__fish_bop_needs_command" -f -a "policy" -d 'Run policy gates'
complete -c bop -n "__fish_bop_needs_command" -f -a "doctor" -d 'Check local toolchain/environment prerequisites'
complete -c bop -n "__fish_bop_needs_command" -f -a "generate-completion" -d 'Generate shell completion script'
complete -c bop -n "__fish_bop_needs_command" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c bop -n "__fish_bop_using_subcommand init" -s h -l help -d 'Print help'
complete -c bop -n "__fish_bop_using_subcommand new" -s h -l help -d 'Print help'
complete -c bop -n "__fish_bop_using_subcommand create" -l from-description -d 'Plain-language task description used to generate the card draft' -r
complete -c bop -n "__fish_bop_using_subcommand create" -l id -d 'Optional explicit id for the generated card' -r
complete -c bop -n "__fish_bop_using_subcommand create" -l yes -d 'Skip confirmation prompt and write draft immediately'
complete -c bop -n "__fish_bop_using_subcommand create" -s h -l help -d 'Print help'
complete -c bop -n "__fish_bop_using_subcommand status" -s h -l help -d 'Print help'
complete -c bop -n "__fish_bop_using_subcommand validate" -l realtime -d 'Run realtime feed validation on the job\'s output records'
complete -c bop -n "__fish_bop_using_subcommand validate" -s h -l help -d 'Print help'
complete -c bop -n "__fish_bop_using_subcommand dispatcher" -l adapter -r
complete -c bop -n "__fish_bop_using_subcommand dispatcher" -l max-workers -r
complete -c bop -n "__fish_bop_using_subcommand dispatcher" -l poll-ms -r
complete -c bop -n "__fish_bop_using_subcommand dispatcher" -l max-retries -r
complete -c bop -n "__fish_bop_using_subcommand dispatcher" -l reap-ms -r
complete -c bop -n "__fish_bop_using_subcommand dispatcher" -l validation-fail-threshold -d 'Error-rate threshold (0.0–1.0) above which a job with critical alerts is moved to failed/ instead of done/. Default 1.0 means never fail' -r
complete -c bop -n "__fish_bop_using_subcommand dispatcher" -l vcs-engine -d 'VCS engine used for workspace preparation and publish' -r -f -a "git_gt\t''
jj\t''"
complete -c bop -n "__fish_bop_using_subcommand dispatcher" -l no-reap
complete -c bop -n "__fish_bop_using_subcommand dispatcher" -l once
complete -c bop -n "__fish_bop_using_subcommand dispatcher" -s h -l help -d 'Print help'
complete -c bop -n "__fish_bop_using_subcommand merge-gate" -l poll-ms -r
complete -c bop -n "__fish_bop_using_subcommand merge-gate" -l vcs-engine -d 'VCS engine used for finalize/publish flow' -r -f -a "git_gt\t''
jj\t''"
complete -c bop -n "__fish_bop_using_subcommand merge-gate" -l once
complete -c bop -n "__fish_bop_using_subcommand merge-gate" -s h -l help -d 'Print help'
complete -c bop -n "__fish_bop_using_subcommand retry" -s h -l help -d 'Print help'
complete -c bop -n "__fish_bop_using_subcommand kill" -s h -l help -d 'Print help'
complete -c bop -n "__fish_bop_using_subcommand approve" -s h -l help -d 'Print help'
complete -c bop -n "__fish_bop_using_subcommand logs" -s f -l follow -d 'Keep streaming as new output arrives (like tail -f)'
complete -c bop -n "__fish_bop_using_subcommand logs" -s h -l help -d 'Print help'
complete -c bop -n "__fish_bop_using_subcommand inspect" -s h -l help -d 'Print help'
complete -c bop -n "__fish_bop_using_subcommand memory; and not __fish_seen_subcommand_from list get set delete help" -s h -l help -d 'Print help'
complete -c bop -n "__fish_bop_using_subcommand memory; and not __fish_seen_subcommand_from list get set delete help" -f -a "list" -d 'List all memory entries in a namespace'
complete -c bop -n "__fish_bop_using_subcommand memory; and not __fish_seen_subcommand_from list get set delete help" -f -a "get" -d 'Get a single memory entry value by key'
complete -c bop -n "__fish_bop_using_subcommand memory; and not __fish_seen_subcommand_from list get set delete help" -f -a "set" -d 'Set a memory entry with a TTL'
complete -c bop -n "__fish_bop_using_subcommand memory; and not __fish_seen_subcommand_from list get set delete help" -f -a "delete" -d 'Delete a memory entry by key'
complete -c bop -n "__fish_bop_using_subcommand memory; and not __fish_seen_subcommand_from list get set delete help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c bop -n "__fish_bop_using_subcommand memory; and __fish_seen_subcommand_from list" -s h -l help -d 'Print help'
complete -c bop -n "__fish_bop_using_subcommand memory; and __fish_seen_subcommand_from get" -s h -l help -d 'Print help'
complete -c bop -n "__fish_bop_using_subcommand memory; and __fish_seen_subcommand_from set" -l ttl-seconds -r
complete -c bop -n "__fish_bop_using_subcommand memory; and __fish_seen_subcommand_from set" -s h -l help -d 'Print help'
complete -c bop -n "__fish_bop_using_subcommand memory; and __fish_seen_subcommand_from delete" -s h -l help -d 'Print help'
complete -c bop -n "__fish_bop_using_subcommand memory; and __fish_seen_subcommand_from help" -f -a "list" -d 'List all memory entries in a namespace'
complete -c bop -n "__fish_bop_using_subcommand memory; and __fish_seen_subcommand_from help" -f -a "get" -d 'Get a single memory entry value by key'
complete -c bop -n "__fish_bop_using_subcommand memory; and __fish_seen_subcommand_from help" -f -a "set" -d 'Set a memory entry with a TTL'
complete -c bop -n "__fish_bop_using_subcommand memory; and __fish_seen_subcommand_from help" -f -a "delete" -d 'Delete a memory entry by key'
complete -c bop -n "__fish_bop_using_subcommand memory; and __fish_seen_subcommand_from help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c bop -n "__fish_bop_using_subcommand serve" -l port -d 'Port to listen on' -r
complete -c bop -n "__fish_bop_using_subcommand serve" -l bind -d 'Bind host or IP (default localhost). WARNING: non-localhost exposes unauthenticated job control endpoints' -r
complete -c bop -n "__fish_bop_using_subcommand serve" -l ui -d 'Serve the browser dashboard at /ui'
complete -c bop -n "__fish_bop_using_subcommand serve" -s h -l help -d 'Print help'
complete -c bop -n "__fish_bop_using_subcommand worktree; and not __fish_seen_subcommand_from list create clean help" -s h -l help -d 'Print help'
complete -c bop -n "__fish_bop_using_subcommand worktree; and not __fish_seen_subcommand_from list create clean help" -f -a "list" -d 'List all job card worktrees and flag orphans'
complete -c bop -n "__fish_bop_using_subcommand worktree; and not __fish_seen_subcommand_from list create clean help" -f -a "create" -d 'Create a git worktree for a pending or running job card'
complete -c bop -n "__fish_bop_using_subcommand worktree; and not __fish_seen_subcommand_from list create clean help" -f -a "clean" -d 'Remove worktrees for done/merged cards or orphaned git worktrees'
complete -c bop -n "__fish_bop_using_subcommand worktree; and not __fish_seen_subcommand_from list create clean help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c bop -n "__fish_bop_using_subcommand worktree; and __fish_seen_subcommand_from list" -s h -l help -d 'Print help'
complete -c bop -n "__fish_bop_using_subcommand worktree; and __fish_seen_subcommand_from create" -s h -l help -d 'Print help'
complete -c bop -n "__fish_bop_using_subcommand worktree; and __fish_seen_subcommand_from clean" -l dry-run
complete -c bop -n "__fish_bop_using_subcommand worktree; and __fish_seen_subcommand_from clean" -s h -l help -d 'Print help'
complete -c bop -n "__fish_bop_using_subcommand worktree; and __fish_seen_subcommand_from help" -f -a "list" -d 'List all job card worktrees and flag orphans'
complete -c bop -n "__fish_bop_using_subcommand worktree; and __fish_seen_subcommand_from help" -f -a "create" -d 'Create a git worktree for a pending or running job card'
complete -c bop -n "__fish_bop_using_subcommand worktree; and __fish_seen_subcommand_from help" -f -a "clean" -d 'Remove worktrees for done/merged cards or orphaned git worktrees'
complete -c bop -n "__fish_bop_using_subcommand worktree; and __fish_seen_subcommand_from help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c bop -n "__fish_bop_using_subcommand providers; and not __fish_seen_subcommand_from list add remove status help" -s h -l help -d 'Print help'
complete -c bop -n "__fish_bop_using_subcommand providers; and not __fish_seen_subcommand_from list add remove status help" -f -a "list" -d 'List all configured providers'
complete -c bop -n "__fish_bop_using_subcommand providers; and not __fish_seen_subcommand_from list add remove status help" -f -a "add" -d 'Add a new provider'
complete -c bop -n "__fish_bop_using_subcommand providers; and not __fish_seen_subcommand_from list add remove status help" -f -a "remove" -d 'Remove a provider'
complete -c bop -n "__fish_bop_using_subcommand providers; and not __fish_seen_subcommand_from list add remove status help" -f -a "status" -d 'Show per-provider job statistics'
complete -c bop -n "__fish_bop_using_subcommand providers; and not __fish_seen_subcommand_from list add remove status help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c bop -n "__fish_bop_using_subcommand providers; and __fish_seen_subcommand_from list" -s h -l help -d 'Print help'
complete -c bop -n "__fish_bop_using_subcommand providers; and __fish_seen_subcommand_from add" -l adapter -r
complete -c bop -n "__fish_bop_using_subcommand providers; and __fish_seen_subcommand_from add" -l model -r
complete -c bop -n "__fish_bop_using_subcommand providers; and __fish_seen_subcommand_from add" -s h -l help -d 'Print help'
complete -c bop -n "__fish_bop_using_subcommand providers; and __fish_seen_subcommand_from remove" -l force
complete -c bop -n "__fish_bop_using_subcommand providers; and __fish_seen_subcommand_from remove" -s h -l help -d 'Print help'
complete -c bop -n "__fish_bop_using_subcommand providers; and __fish_seen_subcommand_from status" -s h -l help -d 'Print help'
complete -c bop -n "__fish_bop_using_subcommand providers; and __fish_seen_subcommand_from help" -f -a "list" -d 'List all configured providers'
complete -c bop -n "__fish_bop_using_subcommand providers; and __fish_seen_subcommand_from help" -f -a "add" -d 'Add a new provider'
complete -c bop -n "__fish_bop_using_subcommand providers; and __fish_seen_subcommand_from help" -f -a "remove" -d 'Remove a provider'
complete -c bop -n "__fish_bop_using_subcommand providers; and __fish_seen_subcommand_from help" -f -a "status" -d 'Show per-provider job statistics'
complete -c bop -n "__fish_bop_using_subcommand providers; and __fish_seen_subcommand_from help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c bop -n "__fish_bop_using_subcommand config; and not __fish_seen_subcommand_from get set help" -s h -l help -d 'Print help'
complete -c bop -n "__fish_bop_using_subcommand config; and not __fish_seen_subcommand_from get set help" -f -a "get" -d 'Print the current value of a config key'
complete -c bop -n "__fish_bop_using_subcommand config; and not __fish_seen_subcommand_from get set help" -f -a "set" -d 'Set a config key to a value (writes to the config file)'
complete -c bop -n "__fish_bop_using_subcommand config; and not __fish_seen_subcommand_from get set help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c bop -n "__fish_bop_using_subcommand config; and __fish_seen_subcommand_from get" -s h -l help -d 'Print help'
complete -c bop -n "__fish_bop_using_subcommand config; and __fish_seen_subcommand_from set" -s h -l help -d 'Print help'
complete -c bop -n "__fish_bop_using_subcommand config; and __fish_seen_subcommand_from help" -f -a "get" -d 'Print the current value of a config key'
complete -c bop -n "__fish_bop_using_subcommand config; and __fish_seen_subcommand_from help" -f -a "set" -d 'Set a config key to a value (writes to the config file)'
complete -c bop -n "__fish_bop_using_subcommand config; and __fish_seen_subcommand_from help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c bop -n "__fish_bop_using_subcommand policy; and not __fish_seen_subcommand_from check help" -s h -l help -d 'Print help'
complete -c bop -n "__fish_bop_using_subcommand policy; and not __fish_seen_subcommand_from check help" -f -a "check" -d 'Check policy for staged changes (default) or a specific card directory'
complete -c bop -n "__fish_bop_using_subcommand policy; and not __fish_seen_subcommand_from check help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c bop -n "__fish_bop_using_subcommand policy; and __fish_seen_subcommand_from check" -l staged -d 'Check staged changes in the current git index'
complete -c bop -n "__fish_bop_using_subcommand policy; and __fish_seen_subcommand_from check" -s h -l help -d 'Print help'
complete -c bop -n "__fish_bop_using_subcommand policy; and __fish_seen_subcommand_from help" -f -a "check" -d 'Check policy for staged changes (default) or a specific card directory'
complete -c bop -n "__fish_bop_using_subcommand policy; and __fish_seen_subcommand_from help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c bop -n "__fish_bop_using_subcommand doctor" -s h -l help -d 'Print help'
complete -c bop -n "__fish_bop_using_subcommand generate-completion" -s h -l help -d 'Print help'
complete -c bop -n "__fish_bop_using_subcommand help; and not __fish_seen_subcommand_from init new create status validate dispatcher merge-gate retry kill approve logs inspect memory serve worktree providers config policy doctor generate-completion help" -f -a "init"
complete -c bop -n "__fish_bop_using_subcommand help; and not __fish_seen_subcommand_from init new create status validate dispatcher merge-gate retry kill approve logs inspect memory serve worktree providers config policy doctor generate-completion help" -f -a "new"
complete -c bop -n "__fish_bop_using_subcommand help; and not __fish_seen_subcommand_from init new create status validate dispatcher merge-gate retry kill approve logs inspect memory serve worktree providers config policy doctor generate-completion help" -f -a "create" -d 'Create a new job draft from a natural-language description'
complete -c bop -n "__fish_bop_using_subcommand help; and not __fish_seen_subcommand_from init new create status validate dispatcher merge-gate retry kill approve logs inspect memory serve worktree providers config policy doctor generate-completion help" -f -a "status"
complete -c bop -n "__fish_bop_using_subcommand help; and not __fish_seen_subcommand_from init new create status validate dispatcher merge-gate retry kill approve logs inspect memory serve worktree providers config policy doctor generate-completion help" -f -a "validate"
complete -c bop -n "__fish_bop_using_subcommand help; and not __fish_seen_subcommand_from init new create status validate dispatcher merge-gate retry kill approve logs inspect memory serve worktree providers config policy doctor generate-completion help" -f -a "dispatcher"
complete -c bop -n "__fish_bop_using_subcommand help; and not __fish_seen_subcommand_from init new create status validate dispatcher merge-gate retry kill approve logs inspect memory serve worktree providers config policy doctor generate-completion help" -f -a "merge-gate"
complete -c bop -n "__fish_bop_using_subcommand help; and not __fish_seen_subcommand_from init new create status validate dispatcher merge-gate retry kill approve logs inspect memory serve worktree providers config policy doctor generate-completion help" -f -a "retry" -d 'Move a card back to pending/ so the dispatcher picks it up again'
complete -c bop -n "__fish_bop_using_subcommand help; and not __fish_seen_subcommand_from init new create status validate dispatcher merge-gate retry kill approve logs inspect memory serve worktree providers config policy doctor generate-completion help" -f -a "kill" -d 'Send SIGTERM to the running agent and mark the card as failed'
complete -c bop -n "__fish_bop_using_subcommand help; and not __fish_seen_subcommand_from init new create status validate dispatcher merge-gate retry kill approve logs inspect memory serve worktree providers config policy doctor generate-completion help" -f -a "approve" -d 'Approve a card that has decision_required set, unblocking it for dispatch'
complete -c bop -n "__fish_bop_using_subcommand help; and not __fish_seen_subcommand_from init new create status validate dispatcher merge-gate retry kill approve logs inspect memory serve worktree providers config policy doctor generate-completion help" -f -a "logs" -d 'Stream stdout and stderr logs for a card'
complete -c bop -n "__fish_bop_using_subcommand help; and not __fish_seen_subcommand_from init new create status validate dispatcher merge-gate retry kill approve logs inspect memory serve worktree providers config policy doctor generate-completion help" -f -a "inspect" -d 'Show meta, spec, and a log summary for a card'
complete -c bop -n "__fish_bop_using_subcommand help; and not __fish_seen_subcommand_from init new create status validate dispatcher merge-gate retry kill approve logs inspect memory serve worktree providers config policy doctor generate-completion help" -f -a "memory" -d 'Manage per-template persistent memory'
complete -c bop -n "__fish_bop_using_subcommand help; and not __fish_seen_subcommand_from init new create status validate dispatcher merge-gate retry kill approve logs inspect memory serve worktree providers config policy doctor generate-completion help" -f -a "serve" -d 'Start the REST API server for CI/CD integration'
complete -c bop -n "__fish_bop_using_subcommand help; and not __fish_seen_subcommand_from init new create status validate dispatcher merge-gate retry kill approve logs inspect memory serve worktree providers config policy doctor generate-completion help" -f -a "worktree" -d 'Manage git worktrees associated with job cards'
complete -c bop -n "__fish_bop_using_subcommand help; and not __fish_seen_subcommand_from init new create status validate dispatcher merge-gate retry kill approve logs inspect memory serve worktree providers config policy doctor generate-completion help" -f -a "providers" -d 'Manage AI providers (list, add, remove, status)'
complete -c bop -n "__fish_bop_using_subcommand help; and not __fish_seen_subcommand_from init new create status validate dispatcher merge-gate retry kill approve logs inspect memory serve worktree providers config policy doctor generate-completion help" -f -a "config" -d 'Read and write global/project config settings'
complete -c bop -n "__fish_bop_using_subcommand help; and not __fish_seen_subcommand_from init new create status validate dispatcher merge-gate retry kill approve logs inspect memory serve worktree providers config policy doctor generate-completion help" -f -a "policy" -d 'Run policy gates'
complete -c bop -n "__fish_bop_using_subcommand help; and not __fish_seen_subcommand_from init new create status validate dispatcher merge-gate retry kill approve logs inspect memory serve worktree providers config policy doctor generate-completion help" -f -a "doctor" -d 'Check local toolchain/environment prerequisites'
complete -c bop -n "__fish_bop_using_subcommand help; and not __fish_seen_subcommand_from init new create status validate dispatcher merge-gate retry kill approve logs inspect memory serve worktree providers config policy doctor generate-completion help" -f -a "generate-completion" -d 'Generate shell completion script'
complete -c bop -n "__fish_bop_using_subcommand help; and not __fish_seen_subcommand_from init new create status validate dispatcher merge-gate retry kill approve logs inspect memory serve worktree providers config policy doctor generate-completion help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c bop -n "__fish_bop_using_subcommand help; and __fish_seen_subcommand_from memory" -f -a "list" -d 'List all memory entries in a namespace'
complete -c bop -n "__fish_bop_using_subcommand help; and __fish_seen_subcommand_from memory" -f -a "get" -d 'Get a single memory entry value by key'
complete -c bop -n "__fish_bop_using_subcommand help; and __fish_seen_subcommand_from memory" -f -a "set" -d 'Set a memory entry with a TTL'
complete -c bop -n "__fish_bop_using_subcommand help; and __fish_seen_subcommand_from memory" -f -a "delete" -d 'Delete a memory entry by key'
complete -c bop -n "__fish_bop_using_subcommand help; and __fish_seen_subcommand_from worktree" -f -a "list" -d 'List all job card worktrees and flag orphans'
complete -c bop -n "__fish_bop_using_subcommand help; and __fish_seen_subcommand_from worktree" -f -a "create" -d 'Create a git worktree for a pending or running job card'
complete -c bop -n "__fish_bop_using_subcommand help; and __fish_seen_subcommand_from worktree" -f -a "clean" -d 'Remove worktrees for done/merged cards or orphaned git worktrees'
complete -c bop -n "__fish_bop_using_subcommand help; and __fish_seen_subcommand_from providers" -f -a "list" -d 'List all configured providers'
complete -c bop -n "__fish_bop_using_subcommand help; and __fish_seen_subcommand_from providers" -f -a "add" -d 'Add a new provider'
complete -c bop -n "__fish_bop_using_subcommand help; and __fish_seen_subcommand_from providers" -f -a "remove" -d 'Remove a provider'
complete -c bop -n "__fish_bop_using_subcommand help; and __fish_seen_subcommand_from providers" -f -a "status" -d 'Show per-provider job statistics'
complete -c bop -n "__fish_bop_using_subcommand help; and __fish_seen_subcommand_from config" -f -a "get" -d 'Print the current value of a config key'
complete -c bop -n "__fish_bop_using_subcommand help; and __fish_seen_subcommand_from config" -f -a "set" -d 'Set a config key to a value (writes to the config file)'
complete -c bop -n "__fish_bop_using_subcommand help; and __fish_seen_subcommand_from policy" -f -a "check" -d 'Check policy for staged changes (default) or a specific card directory'
