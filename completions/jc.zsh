#compdef jc

autoload -U is-at-least

_jc() {
    typeset -A opt_args
    typeset -a _arguments_options
    local ret=1

    if is-at-least 5.2; then
        _arguments_options=(-s -S -C)
    else
        _arguments_options=(-s -C)
    fi

    local context curcontext="$curcontext" state line
    _arguments "${_arguments_options[@]}" : \
'--cards-dir=[]:CARDS_DIR:_default' \
'-h[Print help]' \
'--help[Print help]' \
":: :_jc_commands" \
"*::: :->jc" \
&& ret=0
    case $state in
    (jc)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:jc-command-$line[1]:"
        case $line[1] in
            (init)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(new)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
':template:_default' \
':id:_default' \
&& ret=0
;;
(status)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
'::id:_default' \
&& ret=0
;;
(validate)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
':id:_default' \
&& ret=0
;;
(dispatcher)
_arguments "${_arguments_options[@]}" : \
'--adapter=[]:ADAPTER:_default' \
'--max-workers=[]:MAX_WORKERS:_default' \
'--poll-ms=[]:POLL_MS:_default' \
'--max-retries=[]:MAX_RETRIES:_default' \
'--reap-ms=[]:REAP_MS:_default' \
'--no-reap[]' \
'--once[]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(merge-gate)
_arguments "${_arguments_options[@]}" : \
'--poll-ms=[]:POLL_MS:_default' \
'--once[]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(retry)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
':id:_default' \
&& ret=0
;;
(kill)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
':id:_default' \
&& ret=0
;;
(logs)
_arguments "${_arguments_options[@]}" : \
'-f[Keep streaming as new output arrives (like tail -f)]' \
'--follow[Keep streaming as new output arrives (like tail -f)]' \
'-h[Print help]' \
'--help[Print help]' \
':id:_default' \
&& ret=0
;;
(inspect)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
':id:_default' \
&& ret=0
;;
(completions)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
':shell -- Shell to generate completions for:(bash elvish fish powershell zsh)' \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_jc__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:jc-help-command-$line[1]:"
        case $line[1] in
            (init)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(new)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(status)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(validate)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(dispatcher)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(merge-gate)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(retry)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(kill)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(logs)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(inspect)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(completions)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
        esac
    ;;
esac
}

(( $+functions[_jc_commands] )) ||
_jc_commands() {
    local commands; commands=(
'init:' \
'new:' \
'status:' \
'validate:' \
'dispatcher:' \
'merge-gate:' \
'retry:Move a card back to pending/ so the dispatcher picks it up again' \
'kill:Send SIGTERM to the running agent and mark the card as failed' \
'logs:Stream stdout and stderr logs for a card' \
'inspect:Show meta, spec, and a log summary for a card' \
'completions:Generate shell completions' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'jc commands' commands "$@"
}
(( $+functions[_jc__completions_commands] )) ||
_jc__completions_commands() {
    local commands; commands=()
    _describe -t commands 'jc completions commands' commands "$@"
}
(( $+functions[_jc__dispatcher_commands] )) ||
_jc__dispatcher_commands() {
    local commands; commands=()
    _describe -t commands 'jc dispatcher commands' commands "$@"
}
(( $+functions[_jc__help_commands] )) ||
_jc__help_commands() {
    local commands; commands=(
'init:' \
'new:' \
'status:' \
'validate:' \
'dispatcher:' \
'merge-gate:' \
'retry:Move a card back to pending/ so the dispatcher picks it up again' \
'kill:Send SIGTERM to the running agent and mark the card as failed' \
'logs:Stream stdout and stderr logs for a card' \
'inspect:Show meta, spec, and a log summary for a card' \
'completions:Generate shell completions' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'jc help commands' commands "$@"
}
(( $+functions[_jc__help__completions_commands] )) ||
_jc__help__completions_commands() {
    local commands; commands=()
    _describe -t commands 'jc help completions commands' commands "$@"
}
(( $+functions[_jc__help__dispatcher_commands] )) ||
_jc__help__dispatcher_commands() {
    local commands; commands=()
    _describe -t commands 'jc help dispatcher commands' commands "$@"
}
(( $+functions[_jc__help__help_commands] )) ||
_jc__help__help_commands() {
    local commands; commands=()
    _describe -t commands 'jc help help commands' commands "$@"
}
(( $+functions[_jc__help__init_commands] )) ||
_jc__help__init_commands() {
    local commands; commands=()
    _describe -t commands 'jc help init commands' commands "$@"
}
(( $+functions[_jc__help__inspect_commands] )) ||
_jc__help__inspect_commands() {
    local commands; commands=()
    _describe -t commands 'jc help inspect commands' commands "$@"
}
(( $+functions[_jc__help__kill_commands] )) ||
_jc__help__kill_commands() {
    local commands; commands=()
    _describe -t commands 'jc help kill commands' commands "$@"
}
(( $+functions[_jc__help__logs_commands] )) ||
_jc__help__logs_commands() {
    local commands; commands=()
    _describe -t commands 'jc help logs commands' commands "$@"
}
(( $+functions[_jc__help__merge-gate_commands] )) ||
_jc__help__merge-gate_commands() {
    local commands; commands=()
    _describe -t commands 'jc help merge-gate commands' commands "$@"
}
(( $+functions[_jc__help__new_commands] )) ||
_jc__help__new_commands() {
    local commands; commands=()
    _describe -t commands 'jc help new commands' commands "$@"
}
(( $+functions[_jc__help__retry_commands] )) ||
_jc__help__retry_commands() {
    local commands; commands=()
    _describe -t commands 'jc help retry commands' commands "$@"
}
(( $+functions[_jc__help__status_commands] )) ||
_jc__help__status_commands() {
    local commands; commands=()
    _describe -t commands 'jc help status commands' commands "$@"
}
(( $+functions[_jc__help__validate_commands] )) ||
_jc__help__validate_commands() {
    local commands; commands=()
    _describe -t commands 'jc help validate commands' commands "$@"
}
(( $+functions[_jc__init_commands] )) ||
_jc__init_commands() {
    local commands; commands=()
    _describe -t commands 'jc init commands' commands "$@"
}
(( $+functions[_jc__inspect_commands] )) ||
_jc__inspect_commands() {
    local commands; commands=()
    _describe -t commands 'jc inspect commands' commands "$@"
}
(( $+functions[_jc__kill_commands] )) ||
_jc__kill_commands() {
    local commands; commands=()
    _describe -t commands 'jc kill commands' commands "$@"
}
(( $+functions[_jc__logs_commands] )) ||
_jc__logs_commands() {
    local commands; commands=()
    _describe -t commands 'jc logs commands' commands "$@"
}
(( $+functions[_jc__merge-gate_commands] )) ||
_jc__merge-gate_commands() {
    local commands; commands=()
    _describe -t commands 'jc merge-gate commands' commands "$@"
}
(( $+functions[_jc__new_commands] )) ||
_jc__new_commands() {
    local commands; commands=()
    _describe -t commands 'jc new commands' commands "$@"
}
(( $+functions[_jc__retry_commands] )) ||
_jc__retry_commands() {
    local commands; commands=()
    _describe -t commands 'jc retry commands' commands "$@"
}
(( $+functions[_jc__status_commands] )) ||
_jc__status_commands() {
    local commands; commands=()
    _describe -t commands 'jc status commands' commands "$@"
}
(( $+functions[_jc__validate_commands] )) ||
_jc__validate_commands() {
    local commands; commands=()
    _describe -t commands 'jc validate commands' commands "$@"
}

if [ "$funcstack[1]" = "_jc" ]; then
    _jc "$@"
else
    compdef _jc jc
fi
