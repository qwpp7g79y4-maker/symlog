# symlog

This plugin helps Claude use the `symlog` command-line tool. The tool reads and follows Symfony or Monolog log files in the terminal. Secrets in the output are replaced with `***` unless you ask to show them. On a real terminal it can open a text user interface; otherwise it prints plain lines.

## Use it

Install the tool with `cargo install symlog`. This version needs Rust 1.88 or newer. Give Claude a path to a log file and ask it to show errors, follow new lines, or filter by level or channel. There is no default log path and no standard-input support in this version: a file path is required.

## Data

The tool reads the log file you name. It does not change that file. By default it masks passwords, tokens, API keys, bearer headers, and several well-known secret patterns in the displayed output. `--no-redact` shows the original text. This plugin does not send data to any service and stores nothing. Prefer `--plain` when Claude should read the log as text rather than open the interactive interface.
