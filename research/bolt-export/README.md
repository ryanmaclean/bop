# Bolt Export Snapshot

Source host: `studio` (SSH alias in `~/.ssh/config`)
Source app DB: `/Applications/BoltAI.app/Contents/Resources/db.sqlite3`
Exported at: 2026-03-01 (local session time)

## Files

- `assistants.csv`  
  Columns: `id,displayId,icon,name,label,enable,sortOrder,model,temperature,languageCode,instruction`
- `prompts.csv`  
  Columns: `id,displayId,name,prompt,enable,sortOrder`
- `commands.csv`  
  Columns: `id,displayId,assistantId,icon,label,behavior,optimize,model,temperature,systemInstruction,template,enable,sortOrder,useWebSearch,useWebBrowsing,isInstantCommand,enableAltProfile,altLabel,altModel,altBehavior`
- `schema.csv`  
  Table DDL from `sqlite_master`.

## Row Counts

- `assistants.csv`: 32
- `prompts.csv`: 160
- `commands.csv`: 280 CSV lines (templates/instructions include multiline text)

## Reuse Notes

- Persona reuse seed: `assistants.csv` (`icon`, `name`, `instruction`)
- Prompt library seed: `prompts.csv` (`prompt`)
- Command presets seed: `commands.csv` (`systemInstruction`, `template`)
