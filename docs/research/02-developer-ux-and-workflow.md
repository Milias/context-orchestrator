# Context Manager Research Findings: Developer UX & Workflow Design

> Research conducted 2026-03-10 by exploration agent investigating non-linear interfaces,
> work management integration, Rust TUI frameworks, developer pinning, and collaboration.

---

## Topic 1: Jupyter-Cell-Like Interface for LLM Interactions (Non-Linear Conversations)

### Does the idea make sense?

**Yes, strongly.** The research reveals a clear pain point with linear chat: current LLM interfaces force repeated comparison, modification, and copying of previous content, lowering interaction efficiency and increasing cognitive load. Mindalogue's research (published on arXiv) specifically documents that "linear interaction in current LLMs does not allow for flexible exploration."

### How have others done this before?

**Active projects in this space:**

1. **Mindalogue** - An LLM-based interactive system using a "node + canvas" approach with mindmap structure. It shows that developers can clearly understand logical relationships between different parts of a task or system through node-based, non-linear interaction.
   - Source: [Mindalogue: LLM-Powered Nonlinear Interaction for Effective Learning and Task Exploration (arXiv)](https://arxiv.org/html/2410.10570v1)

2. **Google NotebookLM** - Document-centric AI interaction focused on research and analysis. It organizes around Sources panel (manages information), Chat panel (conversational AI with citations), and Studio panel (creates output like study guides and audio overviews).
   - Source: [NotebookLM gets a new look, audio interactivity and a premium version](https://blog.google/innovation-and-ai/models-and-research/google-labs/notebooklm-new-features-december-2024/)

3. **Wolfram Notebook Assistant** - Turns conversational input into precise computational language for execution in Wolfram Language.
   - Source: [Wolfram Notebook Assistant + LLM Kit: Add the Power of AI](https://www.wolfram.com/notebook-assistant-llm-kit/)

4. **Jupyter AI** - Official Project Jupyter community integration with `%%ai` magic commands, spawning whole notebooks, with a built-in chat pane in JupyterLab.
   - Source: [Learn AI coding in Jupyter notebooks with Jupyter AI](https://www.linkedin.com/posts/andrewyng_ai-coding-just-arrived-in-jupyter-notebooks-activity-7391182314397032448-G1gB/)

5. **Runcell** - AI-powered Jupyter notebook assistant that understands notebook structure and suggests relevant Python code.
   - Source: [#1 Jupyter AI Agent - Runcell](https://www.runcell.dev/)

### Possible approaches?

1. **Node-based canvas** (Mindalogue approach) - Each conversation node is independently positionable, allowing developers to visually organize logic.

2. **Cell execution model** (Jupyter approach) - Cells can be re-ordered and have explicit dependencies, allowing non-linear execution paths.

3. **Graph structure with metadata** - Each node carries type information (Question, Answer, Code, Result) allowing filtering of what gets included in context.

4. **Timeline with branching** - Users can fork conversations, explore different paths, then merge learnings back.

### What could go wrong / what are we missing?

**Cognitive overload risks:**
- Non-linear interfaces can overwhelm users if not carefully designed
- Research shows cluttered screens with too many design elements exceed working memory capacity, causing frustration and errors
- Non-intuitive design forces users to think harder, slowing them down
- Decision paralysis when users face too much information or complexity
- Sources: [Ease Cognitive Overload in UX Design (Mailchimp)](https://mailchimp.com/resources/cognitive-overload/), [Cognitive Load (Laws of UX)](https://lawsofux.com/cognitive-load/)

**Missing considerations:**
- **Context ordering matters** - Just because information is available doesn't mean order is irrelevant for LLM comprehension. How do you preserve meaningful sequence in a graph?
- **Visual scaling** - How do you prevent the graph from becoming too large to visualize? Mindalogue uses topological layering, but this needs to be tested at scale.
- **Legacy compatibility** - How do you export/import to standard chat formats?
- **Context window packing** - Which algorithm determines what gets included in the final prompt when context is pulled from scattered graph nodes?

### Red team / green team the idea

**Green team:** This solves a real problem. Linear chat is inefficient for exploratory work. Developers currently maintain external notebooks to track reasoning. Putting this directly in the tool eliminates context switching.

**Red team:**
- Users will hate it if they can't quickly find what they said. Search becomes critical.
- The "perfect graph visualization" doesn't exist at 1000+ nodes. What's the UX when users hit that limit?
- Most developers are trained on linear chat. Adoption friction is real.
- Complexity cost: is the non-linear benefit worth the implementation effort?

---

## Topic 2: Jira-Like Work Management Integrated with AI Context

### Does the idea make sense?

**Maybe, but be careful about scope.** The research shows Linear has eaten significant Jira market share by being simpler and faster. Integrating AI doesn't automatically solve what made Linear successful (speed, simplicity). However, there's merit in tightly coupling work items with AI conversations.

### How have others done this before?

**Linear vs Jira innovations:**

1. **Linear's key innovation** - Local-first architecture with IndexedDB storing the entire database in your browser. Latency is under 50ms vs Jira's 800-3000ms. This is a performance revolution, not a feature revolution.
   - Source: [Linear vs. Jira: Which Platform is Best in 2025? (Monday.com)](https://monday.com/blog/rnd/linear-or-jira/)

2. **Dart** - Described as "the only truly AI-native project management tool" (Y Combinator). Uses chat as the UI with AI agents as first-class collaborators. Can handle tasks across all roles (marketing, design, sales, coding).
   - Source: [Dart: The only truly AI-native project management tool (Y Combinator)](https://www.ycombinator.com/companies/dart)
   - Source: [AI Project Management That Works — Even If You're Not Technical (Dart)](https://www.dartai.com/)

3. **Forecast PSA** - AI-native project and resource management bringing projects, people, and financials into a single connected view.
   - Source: [The 10 Best AI Project Management Tools for 2026 by Forecast](https://www.forecast.app/blog/10-best-ai-project-management-software)

**Integration patterns:**
- Linear integrates with 200+ tools including Slack, Figma, Sentry, GitHub, GitLab
- Most tools don't deeply integrate AI — they wrap it on top
- None yet successfully merge work items with conversation threads at a deep level

### Possible approaches?

1. **Task-as-context-node** - Each Linear task becomes a graph node that can embed conversations, links, and context directly.

2. **Conversation threads as sub-issues** - Rather than task comments, conversations are first-class issue relationships.

3. **AI-native issue creation** - Users chat with AI, which proposes issue structures, breaking down work automatically.

4. **Dual-panel modal** - Show task details on one side, AI conversation about that task on the other, with automatic context injection.

### What could go wrong / what are we missing?

**Scope creep is the obvious danger:**
- You're building a TUI application, not a full PM tool. Keep scope narrow.
- If you try to replicate Linear, you'll lose. If you try to replicate Jira, you'll definitely lose.

**Missing research:**
- How does conversation history help teams coordinate work? (No data yet)
- Does mixing conversations with work items create archival/compliance issues?
- What's the right permission model for shared graphs across team members?

**Integration complexity:**
- GraphQL API for Linear is good, but syncing bidirectionally between your graph and Linear's graph is non-trivial.
- Do you even need this, or is it better to keep work management separate?

### Red team / green team the idea

**Green team:** A context manager that understands work items helps developers focus. "What was I building this for?" is answered by clicking the linked issue.

**Red team:**
- This adds complexity without clear ROI. Keep work management in Linear/Jira, keep coding in your IDE.
- The "perfect integration" doesn't exist. You'll either duplicate data (sync issues) or create a shallow wrapper (not useful).
- This idea is better as a Linear/Jira plugin than a standalone TUI.

---

## Topic 3: TUI (Terminal UI) in Rust for Developer Tools

### Does the idea make sense?

**Yes, with caveats.** TUIs are thriving for developer tools (lazygit, k9s, etc.), and Rust is the clear choice for performance and correctness. However, graph visualization is TUI's weak point.

### What are the best Rust TUI frameworks?

**Current landscape (2025-2026):**

1. **Ratatui** - Lightweight library providing widgets (tables, tabs, scrollbars, gauges, progress bars, charts, sparklines). Works on most platforms via Crossterm. The go-to framework now after being forked from tui-rs in 2023.
   - Source: [ratatui - Rust](https://docs.rs/ratatui/latest/ratatui/)
   - Source: [GitHub - ratatui/ratatui: A Rust crate for cooking up terminal user interfaces](https://github.com/ratatui/ratatui)

2. **TUI-Realm** - Ratatui framework inspired by Elm and React, bringing state management patterns to TUIs.
   - Source: [GitHub - veeso/tui-realm: A ratatui framework to build stateful applications](https://github.com/veeso/tui-realm)

3. **TachyonFX** (2026 addition) - Effects and animation library integrating with Ratatui, supporting 50+ effects (color transformations, text animations, geometric distortions).
   - Source: [Building Terminal Apps with Ratatui (February 2026)](https://dasroot.net/posts/2026/02/building-terminal-apps-with-ratatui/)

4. **Crossterm** - The low-level terminal interaction library that Ratatui uses by default, supporting most platforms.
   - Source: [Creating a TUI in Rust with Ratatui and Crossterm (Medium)](https://raysuliteanu.medium.com/creating-a-tui-in-rust-e284d31983b3)

### What are the limitations of TUI for complex interfaces like graphs and notebooks?

**Critical limitations:**

1. **Graph visualization is fundamentally hard in text mode:**
   - ANSI terminal compatibility failures on Windows cmd.exe
   - High CPU usage on low-end hardware during animations
   - Redrawing rich TUIs is very CPU expensive (no selective updates like modern GUIs)
   - Source: [Do you know any good TUI libraries comparable to GUI alternatives? (Lobsters)](https://lobste.rs/s/b4jhn0/do_you_know_any_good_tui_libraries)

2. **Workarounds exist but are manual:**
   - ASCII/Unicode Graph Engines can replicate Mermaid flowchart value
   - Topological layering (nodes sorted by dependency depth)
   - Orthogonal routing (box-drawing characters for clean paths)
   - But you're building these from scratch — no existing library
   - Source: [I Built a TUI That Makes Rust Code Inspection Feel Like Magic (DEV Community)](https://dev.to/yashksaini/i-built-a-tui-that-makes-rust-code-inspection-feel-like-magic-375k)

3. **State management complexity:**
   - Managing focus, scroll positions, search state, and animations simultaneously is trickier than React state
   - Requires building your own state machine
   - Source: [Text-Based User Interfaces (Applied Go)](https://appliedgo.net/tui/)

4. **The philosophical limitation:**
   - "You're crudely imitating a real GUI with the crippling limitations of a vt220, when you're in an environment that can almost certainly handle a real GUI."
   - However, this isn't always true — remote development, low-bandwidth links, and accessibility benefit from TUI.
   - Source: [Lobsters discussion on TUI limitations](https://lobste.rs/s/b4jhn0/do_you_know_any_good_tui_libraries)

### Success stories of complex TUIs?

**Proven examples:**

1. **Lazygit** - Git terminal UI for interactive rebasing, complex state management across multiple panels. Built in Go, shows TUIs can handle sophisticated workflows.
   - Source: [Lazygit: The terminal UI that makes git actually usable (BytesizeGo)](https://www.bytesizego.com/blog/lazygit-the-terminal-ui-that-makes-git-actually-usable)

2. **K9s** - Kubernetes terminal UI that continuously watches clusters and offers commands for observed resources. Manages complex real-time state.
   - Source: [Essential CLI/TUI Tools for Developers (FreeCodeCamp)](https://www.freecodecamp.org/news/essential-cli-tui-tools-for-developers/)

3. **Beads Viewer** - Graph-aware TUI for issue tracking with PageRank, critical path, kanban, and dependency DAG visualization. Proof that graph TUIs are possible.
   - Source: [GitHub - Dicklesworthstone/beads_viewer](https://github.com/Dicklesworthstone/beads_viewer)

### Should we consider a hybrid approach (TUI + web)?

**Strong yes.** The research shows:

1. **Hybrid frameworks are emerging:**
   - Vue TermUI - Vue.js-based terminal UI framework
   - Textual (Python) - TUI framework inspired by modern web development
   - Fluent Terminal, Hyper - Terminal emulators based on web tech
   - GoTTY - Shares terminal as web app using xterm.js
   - Warp - Terminal with tabs, split panes, AI, Warp Drive snippets
   - Source: [Vue TermUI](https://vue-termui.dev/)
   - Source: [How To Build Beautiful Terminal UIs (TUIs) in JavaScript! (DEV Community)](https://dev.to/sfundomhlungu/how-to-build-beautiful-terminal-uis-tuis-in-javascript-74j)

2. **WebAssembly approach:**
   - Lipgloss ported to WebAssembly (charsm - Charm CLI + Wasm)
   - Allows building TUIs in JavaScript with WASM backend
   - Source: [How to create web-based terminals (DEV Community)](https://dev.to/saisandeepvaddi/how-to-create-web-based-terminals-38d)

3. **Xterm.js for remote rendering:**
   - Many tools now use Xterm.js to share terminal UIs over web
   - Solves the "graph visualization is hard in TUI" problem by rendering to web when needed
   - Source: [Xterm.js](https://xtermjs.org/)

**Recommendation:** Start pure TUI with Ratatui for the core experience, but design with a web rendering backend as an optional escape hatch. This gives you best of both worlds: fast local TUI for snappy interactions, web rendering for complex graphs.

---

## Topic 4: Developer Highlights / Pinning for System Prompt Construction

### Does the idea make sense?

**Absolutely.** This is a direct response to how modern LLM development actually works. Files like CLAUDE.md and .cursorrules are crude solutions to this problem.

### How do current tools handle persistent context?

**Existing patterns:**

1. **CLAUDE.md Files** - Project-specific instructions automatically read by Claude Agent SDK. Hierarchical (most specific/nested path wins). Persistent "memory" across sessions.
   - Source: [The Complete Guide to AI Agent Memory Files (CLAUDE.md, AGENTS.md, and Beyond)](https://medium.com/data-science-collective/the-complete-guide-to-ai-agent-memory-files-claude-md-agents-md-and-beyond-49ea0df5c5a9)

2. **.cursorrules** - Cursor's equivalent. User-defined instruction sets appended to the system prompt.
   - Source: [Optimizing Coding Agent Rules (./clinerules) for Improved Accuracy (Arize AI)](https://arize.com/blog/optimizing-coding-agent-rules-claude-md-agents-md-clinerules-cursor-rules-for-improved-accuracy/)

3. **AGENTS.md** - The emerging standard (every tool except Claude Code has rallied behind this).
   - Source: [Keep your AGENTS.md in sync — One Source of Truth for AI Instructions](https://kau.sh/blog/agents-md/)

4. **Windsurf's Memories system:**
   - User-generated memories (rules explicitly defined)
   - Automatically generated memories (created by Cascade based on interactions)
   - Ensures continuity across conversations
   - Source: [Windsurf launches surprise in-house AI model family for developers](https://www.therundown.ai/p/windsurfs-surprise-ai-model-reveal)

**Key insight:** Current tools treat context as static files. Your idea of dynamic pinning/unpinning is more sophisticated.

### How does LLM context engineering work?

**Important research on context engineering:**

1. **Context engineering definition** - "The art and science of filling the context window with just the right information for the next step." It's iterative and happens at each prompt turn.
   - Source: [Context Engineering in LLM-Based Agents (Medium)](https://jtanruan.medium.com/context-engineering-in-llm-based-agents-d670d6b439bc)
   - Source: [Effective context engineering for AI agents (Anthropic)](https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents)

2. **System prompts fade over time** - Don't assume a system instruction from prompt #1 carries weight by prompt #20. LLMs only use what fits in the context window, and older turns are dropped first.
   - Source: [Why Your LLM Forgets Your Code After 10 Prompts (DEV Community)](https://dev.to/yaseen_tech/why-your-llm-forgets-your-code-after-10-prompts-and-how-to-fix-context-drift-2hak)

3. **Maintaining context across turns** - Dynamically re-inject core objectives into each turn. Track key facts and current state, reinject into each turn like a mini knowledge base between turns.
   - Source: [LLM Prompt Best Practices for Large Context Windows](https://winder.ai/llm-prompt-best-practices-for-large-context-windows/)

4. **External memory solutions** - Write important details to external store (file, database, state object) instead of packing into prompt. Even if conversational context gets truncated, key info persists and can be re-injected.
   - Source: [The Ultimate Guide to LLM Memory (Medium)](https://medium.com/@sonitanishk2003/the-ultimate-guide-to-llm-memory-from-context-windows-to-advanced-agent-memory-systems-3ec106d2a345)

### What's the right UX for letting developers control what's "always in context"?

**Key principles:**

1. **Visual distinction** - Pinned nodes should look visually different (star icon, highlight color, special section).

2. **Explicit cost display** - Show how many tokens each pinned item consumes. Developers need to understand the trade-off (more pinned = less room for new context).

3. **Automatic unpinning when stale** - Optionally, un-pin items that haven't been referenced in N turns. Prevent accumulation.

4. **Pinning categories:**
   - **System-level** (project rules, coding style - always included)
   - **Conversation-level** (key decisions from this session - include by default)
   - **Temporary** (currently debugging this file - auto-unpin after session)

### Risk of context pollution from too many pinned items?

**Very real risk:**

1. **Token budget constraint** - Most models (Claude 3.5 Sonnet) have 200k context, but not all is available. Pinning 50KB of system context leaves less room for actual conversation.

2. **Diminishing returns** - Beyond ~20% of context as "always included", you're just adding noise. LLM performance degrades.

3. **Maintenance burden** - Stale pinned items become cargo cult rules ("we always pin this, but why?").

**Mitigation strategies:**
- Token counter UI built-in
- Warnings when pinned items exceed 20% of available context
- Auto-cleanup of unused pins after 7 days
- Regular review prompts ("You have 5 pinned items. Review them?")

---

## Topic 5: Local vs Remote Operation (Collaborative AI Coding)

### Does the idea make sense?

**Yes, but carefully scoped.** The research shows collaborative AI coding is growing, but the sync challenges for graph-based data are non-trivial.

### What are the sync challenges for graph-based data?

**Documented challenges from research:**

1. **Scale and performance:**
   - Graphs with millions/billions of vertices and edges present significant technical challenges
   - Visualization tools struggle with exceedingly large graphs (layout algorithms fail)
   - Source: [5 Reasons Graph Data Projects Fail (Gemini Data)](https://www.geminidata.com/5-reasons-graph-data-projects-fail/)

2. **Desktop-first limitations:**
   - Many graph data packages are desktop/client-based, single-user applications
   - Collaboration and sharing views with colleagues is difficult
   - Workaround: create "snapshots" and share via links (like Google Drive)
   - Source: [5 Reasons Graph Data Projects Fail](https://www.geminidata.com/5-reasons-graph-data-projects-fail/)

3. **Time-dependent sync:**
   - Measuring team collaboration dynamics depends on time frame when graph was built
   - Adds temporal complexity to keeping collaborative graphs synchronized
   - Source: [Building the Collaboration Graph of Open-Source Software Ecosystem (arXiv)](https://arxiv.org/abs/2103.12168)

4. **Integration and learning curve:**
   - Introducing graph technology requires careful planning and team alignment
   - Managing effective collaboration and information sharing is hard
   - Source: [5 Reasons Graph Data Projects Fail](https://www.geminidata.com/5-reasons-graph-data-projects-fail/)

### CRDTs for collaborative editing?

**Yes, but be realistic about complexity:**

1. **What are CRDTs:**
   - Data structures replicated across multiple computers where application can update any replica independently
   - An algorithm automatically resolves inconsistencies. Replicas may differ temporarily but eventually converge.
   - Two approaches: state-based (send full state, merge) vs operation-based (send operations, apply independently)
   - Source: [What are CRDTs (Loro)](https://loro.dev/docs/concepts/crdt)
   - Source: [Understanding real-time collaboration with CRDTs (Medium)](https://shambhavishandilya.medium.com/understanding-real-time-collaboration-with-crdts-e764eb65024e)

2. **JSON CRDT specifics:**
   - Conflict-free merged JSON datatype resolves concurrent modifications automatically
   - Supports arbitrarily nested lists and maps modified by insertion, deletion, assignment
   - Multi-value registers preserve concurrent updates when merge is ambiguous
   - Add-wins semantics: addition wins when one replica updates and another deletes
   - Source: [A Conflict-Free Replicated JSON Datatype (arXiv)](https://arxiv.org/pdf/1608.03960)
   - Source: [Operation-based CRDTs: JSON document](https://www.bartoszsypytkowski.com/operation-based-crdts-json-document/)

3. **Real-world usage:**
   - Google Docs, Bet365, League of Legends, PayPal, Redis, Riak, Cosmos DB all use CRDTs
   - Source: [Code - Conflict-free Replicated Data Types](https://crdt.tech/implementations)

4. **Graph-specific libraries:**
   - **Loro** - CRDT library based on Replayable Event Graph, supporting rich text, list, map, movable tree. Rust with JS bindings.
   - **m-ld** - Synchronizes decentralized JSON-LD graph data with query API and pluggable networking/persistence
   - **yjs** - Shared data types library widely used in collaborative software
   - Source: [GitHub - yjs/yjs: Shared data types for building collaborative software](https://github.com/yjs/yjs)

**Key insight:** CRDTs solve *eventual consistency*, not *simultaneous editing*. Two users pinning the same node doesn't create a conflict (order is eventually agreed). But managing "who owns this node" is a different problem.

### Collaborative AI coding tools - what exists?

**Current market (2025-2026):**

1. **GitHub Copilot at scale:**
   - Teams use shared context from docs and repositories
   - 75% higher job satisfaction for users vs non-users
   - 55% more productive at writing code (no quality sacrifice)
   - Source: [GitHub Copilot - Your AI pair programmer](https://github.com/features/copilot)

2. **Cursor for teams:**
   - Supports shared context, chat threads, model presets for consistency
   - Source: [Best AI Tools for Cross-Team Collaboration (Coworker.ai)](https://coworker.ai/blog/best-ai-tools-for-cross-team-collaboration)

3. **Tabnine:**
   - Learns from codebase and team patterns
   - Contextual suggestions enforcing coding standards
   - Source: [Top 15 AI Coding Assistant Tools to Try in 2026 (qodo.ai)](https://www.qodo.ai/blog/best-ai-coding-assistant-tools/)

4. **Duckly for real-time pairing:**
   - Share code and collaborate in real-time
   - Multiple developers using different IDEs simultaneously
   - Terminal session sharing with read-only or write access
   - Real-time local server sharing
   - Source: [Real-time Pair Programming with any IDE - Duckly](https://duckly.com/)

5. **Visual Studio Live Share:**
   - Developers share code environment
   - Others join, view code changes, write code together
   - Source: [Remote Pair Programming 101 (FullScale)](https://fullscale.io/blog/remote-pair-programming-tools-techniques/)

6. **Replit Multiplayer:**
   - Multiple users work in same workspace simultaneously
   - Ghostwriter (built-in AI) helps create, edit, debug code
   - Source: [8 best Tools for Coding Collaboration in 2026 (Kuse.ai)](https://www.kuse.ai/blog/workflows-productivity/tools-for-coding-collaboration)

**Research insight:** Collaborative AI coding shows 21% speed increase and 40% faster code review when properly integrated.
   - Source: [How to implement collaborative AI coding in enterprise teams (GetDX)](https://getdx.com/blog/collaborative-ai-coding/)

---

## Synthesis & Red Team / Green Team

### The Strongest Version of This Idea

**What makes sense to build:**

1. **Local-first TUI** (Ratatui + Rust) for building conversation graphs inside the terminal
2. **Node types** - Question, Response, Code, Result - allowing semantic filtering
3. **Cell-like execution** - Run code cells from the graph, results stored as nodes
4. **Pinning system** - Mark important nodes, auto-include in prompts to LLM
5. **Export to prompt** - Select graph nodes, generate optimized prompt for Claude/etc
6. **Git-friendly** - Store graph as JSON, version control friendly

**This solves a real problem:** Developers currently maintain *outside* context (Notion, Obsidian, CLAUDE.md files). Pulling this into the coding loop is genuinely useful.

### Green Team Arguments

1. **Fills a real gap** - No existing tool combines graph conversations + code execution + LLM context management in one place
2. **Composable** - Graph structure makes it easy to experiment (swap nodes, reorder, exclude)
3. **Educational** - Builds intuition about context engineering and token management
4. **Bootstraps easily** - Can start with just conversation nodes, add code execution later
5. **TUI is right call** - Developers live in terminals. Removes friction vs web-based tools.

### Red Team Arguments

1. **Feature creep trap** - You'll feel pressure to add work management, real-time collaboration, complex graph visualization. Resist. These belong in other tools.

2. **UX complexity** - The Mindalogue paper shows non-linear interfaces work, but users struggle to find things. Your search and organization UX needs to be *exceptional*.

3. **LLM context window changes fast** - By the time you ship, Claude may have 1M+ context tokens. The whole "pinning" problem might evaporate.

4. **Competition exists** - Windsurf's Memories, Cursor's shared context, notebooklm's document pinning all attack related problems. What's your unique angle?

5. **Team collaboration is expensive** - If you wait to support teams, you're blocking adoption. If you ship local-only, it feels limited.

### What's the killer test?

**Before building, validate:**

1. **Interview 10 developers** - "Would you use a TUI app that stores your debugging conversations in a graph and lets you selectively include them in prompts?" Get specific use cases.

2. **Prototype in a weekend** - Build minimal proof-of-concept (3-node graph in Ratatui, export to prompt). See if the interaction feels good.

3. **Test with real LLM usage** - Does including pinned nodes actually improve coding task outcomes vs baseline chat? Measure it.

4. **Competitive analysis** - Why is this better than:
   - Windsurf's Memories + Cascade chat
   - Cursor's shared context threads
   - NotebookLM's document pinning
   - User's existing CLAUDE.md + .cursorrules

If you can't articulate why this is distinctly better, the project isn't clear enough yet.

---

## Key Sources Summary

**Topic 1 - Non-linear conversations:**
- Mindalogue research: https://arxiv.org/html/2410.10570v1
- Google NotebookLM: https://blog.google/innovation-and-ai/models-and-research/google-labs/notebooklm-new-features-december-2024/
- Runcell: https://www.runcell.dev/

**Topic 2 - Project management integration:**
- Linear vs Jira: https://monday.com/blog/rnd/linear-or-jira/
- Dart AI-native PM: https://www.dartai.com/

**Topic 3 - Rust TUI:**
- Ratatui docs: https://docs.rs/ratatui/latest/ratatui/
- TachyonFX effects: https://dasroot.net/posts/2026/02/building-terminal-apps-with-ratatui/
- Beads graph TUI: https://github.com/Dicklesworthstone/beads_viewer

**Topic 4 - Context pinning:**
- CLAUDE.md guide: https://medium.com/data-science-collective/the-complete-guide-to-ai-agent-memory-files-claude-md-agents-md-and-beyond-49ea0df5c5a9
- Anthropic context engineering: https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents

**Topic 5 - Collaboration:**
- CRDT primer: https://loro.dev/docs/concepts/crdt
- JSON CRDT paper: https://arxiv.org/pdf/1608.03960
- Duckly for pairing: https://duckly.com/
- Graph sync challenges: https://www.geminidata.com/5-reasons-graph-data-projects-fail/
