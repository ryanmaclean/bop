// Source: /tmp/auto-claude-asar-1772421975/out/renderer/assets/index-3inA-y1N.js (lines 133140-133392)
const AGENT_CONFIGS = {
  // Spec Creation Phases - all use 'spec' phase settings
  spec_gatherer: {
    label: "Spec Gatherer",
    description: "Collects initial requirements from user",
    category: "spec",
    tools: ["Read", "Glob", "Grep", "WebFetch", "WebSearch"],
    mcp_servers: [],
    settingsSource: { type: "phase", phase: "spec" }
  },
  spec_researcher: {
    label: "Spec Researcher",
    description: "Validates external integrations and APIs",
    category: "spec",
    tools: ["Read", "Glob", "Grep", "WebFetch", "WebSearch"],
    mcp_servers: ["context7"],
    settingsSource: { type: "phase", phase: "spec" }
  },
  spec_writer: {
    label: "Spec Writer",
    description: "Creates the spec.md document",
    category: "spec",
    tools: ["Read", "Glob", "Grep", "Write", "Edit", "Bash"],
    mcp_servers: [],
    settingsSource: { type: "phase", phase: "spec" }
  },
  spec_critic: {
    label: "Spec Critic",
    description: "Self-critique using deep analysis",
    category: "spec",
    tools: ["Read", "Glob", "Grep"],
    mcp_servers: [],
    settingsSource: { type: "phase", phase: "spec" }
  },
  spec_discovery: {
    label: "Spec Discovery",
    description: "Initial project discovery and analysis",
    category: "spec",
    tools: ["Read", "Glob", "Grep", "WebFetch", "WebSearch"],
    mcp_servers: [],
    settingsSource: { type: "phase", phase: "spec" }
  },
  spec_context: {
    label: "Spec Context",
    description: "Builds context from existing codebase",
    category: "spec",
    tools: ["Read", "Glob", "Grep"],
    mcp_servers: [],
    settingsSource: { type: "phase", phase: "spec" }
  },
  spec_validation: {
    label: "Spec Validation",
    description: "Validates spec completeness and quality",
    category: "spec",
    tools: ["Read", "Glob", "Grep"],
    mcp_servers: [],
    settingsSource: { type: "phase", phase: "spec" }
  },
  // Build Phases
  planner: {
    label: "Planner",
    description: "Creates implementation plan with subtasks",
    category: "build",
    tools: ["Read", "Glob", "Grep", "Write", "Edit", "Bash", "WebFetch", "WebSearch"],
    mcp_servers: ["context7", "graphiti-memory", "auto-claude"],
    mcp_optional: ["linear"],
    settingsSource: { type: "phase", phase: "planning" }
  },
  coder: {
    label: "Coder",
    description: "Implements individual subtasks",
    category: "build",
    tools: ["Read", "Glob", "Grep", "Write", "Edit", "Bash", "WebFetch", "WebSearch"],
    mcp_servers: ["context7", "graphiti-memory", "auto-claude"],
    mcp_optional: ["linear"],
    settingsSource: { type: "phase", phase: "coding" }
  },
  // QA Phases
  qa_reviewer: {
    label: "QA Reviewer",
    description: "Validates acceptance criteria. Uses Electron or Puppeteer based on project type.",
    category: "qa",
    tools: ["Read", "Glob", "Grep", "Bash", "WebFetch", "WebSearch"],
    mcp_servers: ["context7", "graphiti-memory", "auto-claude"],
    mcp_optional: ["linear", "electron", "puppeteer"],
    settingsSource: { type: "phase", phase: "qa" }
  },
  qa_fixer: {
    label: "QA Fixer",
    description: "Fixes QA-reported issues. Uses Electron or Puppeteer based on project type.",
    category: "qa",
    tools: ["Read", "Glob", "Grep", "Write", "Edit", "Bash", "WebFetch", "WebSearch"],
    mcp_servers: ["context7", "graphiti-memory", "auto-claude"],
    mcp_optional: ["linear", "electron", "puppeteer"],
    settingsSource: { type: "phase", phase: "qa" }
  },
  // Utility Phases - use feature settings
  pr_reviewer: {
    label: "PR Reviewer",
    description: "Reviews GitHub pull requests",
    category: "utility",
    tools: ["Read", "Glob", "Grep", "WebFetch", "WebSearch"],
    mcp_servers: ["context7"],
    settingsSource: { type: "feature", feature: "githubPrs" }
  },
  commit_message: {
    label: "Commit Message",
    description: "Generates commit messages",
    category: "utility",
    tools: [],
    mcp_servers: [],
    settingsSource: { type: "feature", feature: "utility" }
  },
  merge_resolver: {
    label: "Merge Resolver",
    description: "Resolves merge conflicts",
    category: "utility",
    tools: [],
    mcp_servers: [],
    settingsSource: { type: "feature", feature: "utility" }
  },
  insights: {
    label: "Insights",
    description: "Extracts code insights",
    category: "utility",
    tools: ["Read", "Glob", "Grep", "WebFetch", "WebSearch"],
    mcp_servers: [],
    settingsSource: { type: "feature", feature: "insights" }
  },
  analysis: {
    label: "Analysis",
    description: "Codebase analysis with context lookup",
    category: "utility",
    tools: ["Read", "Glob", "Grep", "WebFetch", "WebSearch"],
    mcp_servers: ["context7"],
    // Analysis uses same as insights
    settingsSource: { type: "feature", feature: "insights" }
  },
  batch_analysis: {
    label: "Batch Analysis",
    description: "Batch processing of issues or items",
    category: "utility",
    tools: ["Read", "Glob", "Grep", "WebFetch", "WebSearch"],
    mcp_servers: [],
    // Batch uses same as GitHub Issues
    settingsSource: { type: "feature", feature: "githubIssues" }
  },
  // Ideation & Roadmap - use feature settings
  ideation: {
    label: "Ideation",
    description: "Generates feature ideas",
    category: "ideation",
    tools: ["Read", "Glob", "Grep", "WebFetch", "WebSearch"],
    mcp_servers: [],
    settingsSource: { type: "feature", feature: "ideation" }
  },
  roadmap_discovery: {
    label: "Roadmap Discovery",
    description: "Discovers roadmap items",
    category: "ideation",
    tools: ["Read", "Glob", "Grep", "WebFetch", "WebSearch"],
    mcp_servers: ["context7"],
    settingsSource: { type: "feature", feature: "roadmap" }
  },
  pr_template_filler: {
    label: "PR Template Filler",
    description: "Generates AI-powered PR descriptions from templates",
    category: "utility",
    tools: ["Read", "Glob", "Grep"],
    mcp_servers: [],
    settingsSource: { type: "feature", feature: "utility" }
  }
};
const MCP_SERVERS = {
  context7: {
    name: "Context7",
    description: "Documentation lookup for libraries and frameworks via @upstash/context7-mcp",
    icon: Search,
    tools: ["mcp__context7__resolve-library-id", "mcp__context7__query-docs"]
  },
  "graphiti-memory": {
    name: "Graphiti Memory",
    description: "Knowledge graph for cross-session context. Requires GRAPHITI_MCP_URL env var.",
    icon: Brain,
    tools: [
      "mcp__graphiti-memory__search_nodes",
      "mcp__graphiti-memory__search_facts",
      "mcp__graphiti-memory__add_episode",
      "mcp__graphiti-memory__get_episodes",
      "mcp__graphiti-memory__get_entity_edge"
    ]
  },
  "auto-claude": {
    name: "Auto-Claude Tools",
    description: "Build progress tracking, session context, discoveries & gotchas recording",
    icon: ListChecks,
    tools: [
      "mcp__auto-claude__update_subtask_status",
      "mcp__auto-claude__get_build_progress",
      "mcp__auto-claude__record_discovery",
      "mcp__auto-claude__record_gotcha",
      "mcp__auto-claude__get_session_context",
      "mcp__auto-claude__update_qa_status"
    ]
  },
  linear: {
    name: "Linear",
    description: "Project management via Linear API. Requires LINEAR_API_KEY env var.",
    icon: ClipboardList,
    tools: [
      "mcp__linear-server__list_teams",
      "mcp__linear-server__list_projects",
      "mcp__linear-server__list_issues",
      "mcp__linear-server__create_issue",
      "mcp__linear-server__update_issue"
      // ... and more
    ]
  },
  electron: {
    name: "Electron MCP",
    description: "Desktop app automation via Chrome DevTools Protocol. Requires ELECTRON_MCP_ENABLED=true.",
    icon: Monitor,
    tools: [
      "mcp__electron__get_electron_window_info",
      "mcp__electron__take_screenshot",
      "mcp__electron__send_command_to_electron",
      "mcp__electron__read_electron_logs"
    ]
  },
  puppeteer: {
    name: "Puppeteer MCP",
    description: "Web browser automation for non-Electron web frontends.",
    icon: Globe,
    tools: [
      "mcp__puppeteer__puppeteer_connect_active_tab",
      "mcp__puppeteer__puppeteer_navigate",
      "mcp__puppeteer__puppeteer_screenshot",
      "mcp__puppeteer__puppeteer_click",
      "mcp__puppeteer__puppeteer_fill",
      "mcp__puppeteer__puppeteer_select",
      "mcp__puppeteer__puppeteer_hover",
      "mcp__puppeteer__puppeteer_evaluate"
    ]
  }
};
const ALL_MCP_SERVERS = [
  "context7",
  "graphiti-memory",
  "linear",
  "electron",
  "puppeteer",
  "auto-claude"
];
