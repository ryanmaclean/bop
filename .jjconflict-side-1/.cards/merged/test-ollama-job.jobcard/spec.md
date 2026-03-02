# Implementation Task: Add CLI Command for Job Retry

## Overview
Add a `retry` command to the JobCard CLI that allows users to retry failed jobs by moving them back to the pending queue.

## Requirements
- Add `retry` subcommand to CLI
- Support retrying individual jobs by ID
- Support retrying all failed jobs with `--all` flag
- Reset retry count when retrying
- Update provider chain to rotate providers on retry

## Acceptance Criteria
- [ ] `jc retry <job-id>` moves job from failed/ to pending/
- [ ] `jc retry --all` moves all failed jobs to pending/
- [ ] Retry count is reset to 0
- [ ] Provider chain is rotated
- [ ] Command shows feedback on what was retried
