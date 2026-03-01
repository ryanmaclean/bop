# JobCard Launchd Services

This directory contains launchd plist files for running JobCard services as background processes on macOS.

## Installation

1. Copy the plist files to `~/Library/LaunchAgents/`:
```bash
cp com.yourorg.jobcard.dispatcher.plist ~/Library/LaunchAgents/
cp com.yourorg.jobcard.merge-gate.plist ~/Library/LaunchAgents/
```

2. Load the services:
```bash
launchctl load ~/Library/LaunchAgents/com.yourorg.jobcard.dispatcher.plist
launchctl load ~/Library/LaunchAgents/com.yourorg.jobcard.merge-gate.plist
```

## Management

### Start services:
```bash
launchctl start com.yourorg.jobcard.dispatcher
launchctl start com.yourorg.jobcard.merge-gate
```

### Stop services:
```bash
launchctl stop com.yourorg.jobcard.dispatcher
launchctl stop com.yourorg.jobcard.merge-gate
```

### Unload services:
```bash
launchctl unload ~/Library/LaunchAgents/com.yourorg.jobcard.dispatcher.plist
launchctl unload ~/Library/LaunchAgents/com.yourorg.jobcard.merge-gate.plist
```

### Check status:
```bash
launchctl list | grep jobcard
```

## Logs

- Dispatcher logs: `/tmp/jobcard-dispatcher.log`
- Dispatcher errors: `/tmp/jobcard-dispatcher.err`
- Merge gate logs: `/tmp/jobcard-merge-gate.log`
- Merge gate errors: `/tmp/jobcard-merge-gate.err`

## Configuration

The plist files assume:
- JobCard binary is installed at `/usr/local/bin/jc`
- Working directory is `/Users/studio/gtfs`
- Cards directory is `/Users/studio/gtfs/.cards`

Update these paths in the plist files if your installation differs.
