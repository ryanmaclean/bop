// Source: /tmp/auto-claude-asar-1772421975/out/renderer/assets/index-3inA-y1N.js (lines 362-507)
const AVAILABLE_MODELS = [
  { value: "opus", label: "Claude Opus 4.6" },
  { value: "opus-1m", label: "Claude Opus 4.6 (1M)" },
  { value: "opus-4.5", label: "Claude Opus 4.5" },
  { value: "sonnet", label: "Claude Sonnet 4.5" },
  { value: "haiku", label: "Claude Haiku 4.5" }
];
const THINKING_LEVELS = [
  { value: "low", label: "Low", description: "Brief consideration" },
  { value: "medium", label: "Medium", description: "Moderate analysis" },
  { value: "high", label: "High", description: "Deep thinking" }
];
const AUTO_PHASE_MODELS = {
  spec: "opus",
  planning: "opus",
  coding: "opus",
  qa: "opus"
};
const AUTO_PHASE_THINKING = {
  spec: "high",
  // Deep thinking for comprehensive spec creation
  planning: "high",
  // High thinking for planning complex features
  coding: "low",
  // Faster coding iterations
  qa: "low"
  // Efficient QA review
};
const COMPLEX_PHASE_MODELS = {
  spec: "opus",
  planning: "opus",
  coding: "opus",
  qa: "opus"
};
const COMPLEX_PHASE_THINKING = {
  spec: "high",
  planning: "high",
  coding: "high",
  qa: "high"
};
const BALANCED_PHASE_MODELS = {
  spec: "sonnet",
  planning: "sonnet",
  coding: "sonnet",
  qa: "sonnet"
};
const BALANCED_PHASE_THINKING = {
  spec: "medium",
  planning: "medium",
  coding: "medium",
  qa: "medium"
};
const QUICK_PHASE_MODELS = {
  spec: "haiku",
  planning: "haiku",
  coding: "haiku",
  qa: "haiku"
};
const QUICK_PHASE_THINKING = {
  spec: "low",
  planning: "low",
  coding: "low",
  qa: "low"
};
const DEFAULT_PHASE_MODELS = BALANCED_PHASE_MODELS;
const DEFAULT_PHASE_THINKING = BALANCED_PHASE_THINKING;
const DEFAULT_FEATURE_MODELS = {
  insights: "sonnet",
  // Fast, responsive chat
  ideation: "opus",
  // Creative ideation benefits from Opus
  roadmap: "opus",
  // Strategic planning benefits from Opus
  githubIssues: "opus",
  // Issue triage and analysis benefits from Opus
  githubPrs: "opus",
  // PR review benefits from thorough Opus analysis
  utility: "haiku"
  // Fast utility operations (commit messages, merge resolution)
};
const DEFAULT_FEATURE_THINKING = {
  insights: "medium",
  // Balanced thinking for chat
  ideation: "high",
  // Deep thinking for creative ideas
  roadmap: "high",
  // Strategic thinking for roadmap
  githubIssues: "medium",
  // Moderate thinking for issue analysis
  githubPrs: "medium",
  // Moderate thinking for PR review
  utility: "low"
  // Fast thinking for utility operations
};
const FEATURE_LABELS = {
  insights: { label: "Insights Chat", description: "Ask questions about your codebase" },
  ideation: { label: "Ideation", description: "Generate feature ideas and improvements" },
  roadmap: { label: "Roadmap", description: "Create strategic feature roadmaps" },
  githubIssues: { label: "GitHub Issues", description: "Automated issue triage and labeling" },
  githubPrs: { label: "GitHub PR Review", description: "AI-powered pull request reviews" },
  utility: { label: "Utility", description: "Commit messages and merge conflict resolution" }
};
const DEFAULT_AGENT_PROFILES = [
  {
    id: "auto",
    name: "Auto (Optimized)",
    description: "Uses Opus across all phases with optimized thinking levels",
    model: "opus",
    thinkingLevel: "high",
    icon: "Sparkles",
    phaseModels: AUTO_PHASE_MODELS,
    phaseThinking: AUTO_PHASE_THINKING
  },
  {
    id: "complex",
    name: "Complex Tasks",
    description: "For intricate, multi-step implementations requiring deep analysis",
    model: "opus",
    thinkingLevel: "high",
    icon: "Brain",
    phaseModels: COMPLEX_PHASE_MODELS,
    phaseThinking: COMPLEX_PHASE_THINKING
  },
  {
    id: "balanced",
    name: "Balanced",
    description: "Good balance of speed and quality for most tasks",
    model: "sonnet",
    thinkingLevel: "medium",
    icon: "Scale",
    phaseModels: BALANCED_PHASE_MODELS,
    phaseThinking: BALANCED_PHASE_THINKING
  },
  {
    id: "quick",
    name: "Quick Edits",
    description: "Fast iterations for simple changes and quick fixes",
    model: "haiku",
    thinkingLevel: "low",
    icon: "Zap",
    phaseModels: QUICK_PHASE_MODELS,
    phaseThinking: QUICK_PHASE_THINKING
  }
];
const FAST_MODE_MODELS = ["opus", "opus-1m"];
const ADAPTIVE_THINKING_MODELS = ["opus", "opus-1m"];
