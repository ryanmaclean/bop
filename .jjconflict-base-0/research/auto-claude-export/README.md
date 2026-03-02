# Auto-Claude Export (Prompts, Personas, MCP)

Source app: [Auto-Claude](https://github.com/krzysztofczyk/auto-claude) packaged install at `/Applications/Auto-Claude.app`.

Snapshot metadata is in `source_info.txt`.

## What Was Collected

- **Prompt corpus** from backend markdown prompts (`source/backend/prompts/`)
- **Prompt assembly/runtime files** used to build/inject prompts:
  - `source/backend/prompts_pkg/prompts.py`
  - `source/backend/prompts_pkg/prompt_generator.py`
  - `source/backend/prompts_pkg/project_context.py`
  - `source/backend/prompts.py`
- **Persona/profile presets** (UI agent profiles: auto/complex/balanced/quick) from extracted app bundle snippets
- **MCP registries + agent/MCP mappings** from both runtime backend and UI registry snippets

## Catalog Files

- `prompts_catalog.tsv`
  - Columns: `path,group,bytes,sha256`
- `personas_catalog.tsv`
  - Columns: `profile_id,name,description,icon,default_model,default_thinking,phase model/thinking columns,source`
- `mcp_servers_catalog.tsv`
  - Columns: `server_id,display_name,primary_transport,activation_gate,default_in_agent_configs,tool_count,sources`
- `agent_mcp_matrix.tsv`
  - Columns: `agent_id,category,required_mcp,optional_mcp,source`

## Source Snapshots

- `source/backend/prompts/**` (all `.md` prompts)
- `source/backend/prompts_pkg/*.py`
- `source/backend/agents/tools_pkg/models.py`
- `source/backend/core/client.py`
- `source/ui/default_agent_profiles.snippet.js`
- `source/ui/agent_mcp_registry.snippet.js`

## Notes For Skill Conversion

- **Prompt skill candidates**: split by prompt group (`core`, `github`, `mcp_tools`) from `prompts_catalog.tsv`.
- **Persona skill candidates**: one skill per profile (`auto`, `complex`, `balanced`, `quick`) using `personas_catalog.tsv`.
- **MCP skill candidates**: one skill for server policy + one for agent mapping (from `mcp_servers_catalog.tsv` + `agent_mcp_matrix.tsv`).
- Use backend Python files as the runtime source of truth when UI and backend drift.
