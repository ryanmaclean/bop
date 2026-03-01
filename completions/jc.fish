# Print an optspec for argparse to handle cmd's options that are independent of any subcommand.
function __fish_jc_global_optspecs
	string join \n cards-dir= h/help
end

function __fish_jc_needs_command
	# Figure out if the current invocation already has a command.
	set -l cmd (commandline -opc)
	set -e cmd[1]
	argparse -s (__fish_jc_global_optspecs) -- $cmd 2>/dev/null
	or return
	if set -q argv[1]
		# Also print the command, so this can be used to figure out what it is.
		echo $argv[1]
		return 1
	end
	return 0
end

function __fish_jc_using_subcommand
	set -l cmd (__fish_jc_needs_command)
	test -z "$cmd"
	and return 1
	contains -- $cmd[1] $argv
end

complete -c jc -n "__fish_jc_needs_command" -l cards-dir -r
complete -c jc -n "__fish_jc_needs_command" -s h -l help -d 'Print help'
complete -c jc -n "__fish_jc_needs_command" -f -a "init"
complete -c jc -n "__fish_jc_needs_command" -f -a "new"
complete -c jc -n "__fish_jc_needs_command" -f -a "status"
complete -c jc -n "__fish_jc_needs_command" -f -a "validate"
complete -c jc -n "__fish_jc_needs_command" -f -a "dispatcher"
complete -c jc -n "__fish_jc_needs_command" -f -a "merge-gate"
complete -c jc -n "__fish_jc_needs_command" -f -a "retry" -d 'Move a card back to pending/ so the dispatcher picks it up again'
complete -c jc -n "__fish_jc_needs_command" -f -a "kill" -d 'Send SIGTERM to the running agent and mark the card as failed'
complete -c jc -n "__fish_jc_needs_command" -f -a "logs" -d 'Stream stdout and stderr logs for a card'
complete -c jc -n "__fish_jc_needs_command" -f -a "inspect" -d 'Show meta, spec, and a log summary for a card'
complete -c jc -n "__fish_jc_needs_command" -f -a "completions" -d 'Generate shell completions'
complete -c jc -n "__fish_jc_needs_command" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c jc -n "__fish_jc_using_subcommand init" -s h -l help -d 'Print help'
complete -c jc -n "__fish_jc_using_subcommand new" -s h -l help -d 'Print help'
complete -c jc -n "__fish_jc_using_subcommand status" -s h -l help -d 'Print help'
complete -c jc -n "__fish_jc_using_subcommand validate" -s h -l help -d 'Print help'
complete -c jc -n "__fish_jc_using_subcommand dispatcher" -l adapter -r
complete -c jc -n "__fish_jc_using_subcommand dispatcher" -l max-workers -r
complete -c jc -n "__fish_jc_using_subcommand dispatcher" -l poll-ms -r
complete -c jc -n "__fish_jc_using_subcommand dispatcher" -l max-retries -r
complete -c jc -n "__fish_jc_using_subcommand dispatcher" -l reap-ms -r
complete -c jc -n "__fish_jc_using_subcommand dispatcher" -l no-reap
complete -c jc -n "__fish_jc_using_subcommand dispatcher" -l once
complete -c jc -n "__fish_jc_using_subcommand dispatcher" -s h -l help -d 'Print help'
complete -c jc -n "__fish_jc_using_subcommand merge-gate" -l poll-ms -r
complete -c jc -n "__fish_jc_using_subcommand merge-gate" -l once
complete -c jc -n "__fish_jc_using_subcommand merge-gate" -s h -l help -d 'Print help'
complete -c jc -n "__fish_jc_using_subcommand retry" -s h -l help -d 'Print help'
complete -c jc -n "__fish_jc_using_subcommand kill" -s h -l help -d 'Print help'
complete -c jc -n "__fish_jc_using_subcommand logs" -s f -l follow -d 'Keep streaming as new output arrives (like tail -f)'
complete -c jc -n "__fish_jc_using_subcommand logs" -s h -l help -d 'Print help'
complete -c jc -n "__fish_jc_using_subcommand inspect" -s h -l help -d 'Print help'
complete -c jc -n "__fish_jc_using_subcommand completions" -s h -l help -d 'Print help'
complete -c jc -n "__fish_jc_using_subcommand help; and not __fish_seen_subcommand_from init new status validate dispatcher merge-gate retry kill logs inspect completions help" -f -a "init"
complete -c jc -n "__fish_jc_using_subcommand help; and not __fish_seen_subcommand_from init new status validate dispatcher merge-gate retry kill logs inspect completions help" -f -a "new"
complete -c jc -n "__fish_jc_using_subcommand help; and not __fish_seen_subcommand_from init new status validate dispatcher merge-gate retry kill logs inspect completions help" -f -a "status"
complete -c jc -n "__fish_jc_using_subcommand help; and not __fish_seen_subcommand_from init new status validate dispatcher merge-gate retry kill logs inspect completions help" -f -a "validate"
complete -c jc -n "__fish_jc_using_subcommand help; and not __fish_seen_subcommand_from init new status validate dispatcher merge-gate retry kill logs inspect completions help" -f -a "dispatcher"
complete -c jc -n "__fish_jc_using_subcommand help; and not __fish_seen_subcommand_from init new status validate dispatcher merge-gate retry kill logs inspect completions help" -f -a "merge-gate"
complete -c jc -n "__fish_jc_using_subcommand help; and not __fish_seen_subcommand_from init new status validate dispatcher merge-gate retry kill logs inspect completions help" -f -a "retry" -d 'Move a card back to pending/ so the dispatcher picks it up again'
complete -c jc -n "__fish_jc_using_subcommand help; and not __fish_seen_subcommand_from init new status validate dispatcher merge-gate retry kill logs inspect completions help" -f -a "kill" -d 'Send SIGTERM to the running agent and mark the card as failed'
complete -c jc -n "__fish_jc_using_subcommand help; and not __fish_seen_subcommand_from init new status validate dispatcher merge-gate retry kill logs inspect completions help" -f -a "logs" -d 'Stream stdout and stderr logs for a card'
complete -c jc -n "__fish_jc_using_subcommand help; and not __fish_seen_subcommand_from init new status validate dispatcher merge-gate retry kill logs inspect completions help" -f -a "inspect" -d 'Show meta, spec, and a log summary for a card'
complete -c jc -n "__fish_jc_using_subcommand help; and not __fish_seen_subcommand_from init new status validate dispatcher merge-gate retry kill logs inspect completions help" -f -a "completions" -d 'Generate shell completions'
complete -c jc -n "__fish_jc_using_subcommand help; and not __fish_seen_subcommand_from init new status validate dispatcher merge-gate retry kill logs inspect completions help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
