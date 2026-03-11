# Inline Tool Invocation Patterns

> Research conducted 2026-03-12. Survey of how AI coding assistants and agent
> frameworks handle tool calls triggered from natural language conversations.
> Covers Anthropic Claude, OpenAI, LangChain/LangGraph, AI code editors
> (Cursor, Windsurf, Cline), slash command systems, and MCP.

---

## 1. Introduction

Tool invocation is the mechanism by which an AI system translates natural language intent into concrete actions -- reading files, querying APIs, executing commands, or modifying state. The design of this mechanism determines an agent's autonomy, reliability, and composability.

This document surveys six major systems and categorizes their approaches along a spectrum:

- **LLM-driven**: The model decides which tool to call, with what arguments, based on its understanding of the conversation and available tool schemas. Examples: Claude tool_use, OpenAI function calling, LangChain ReAct agents.
- **Rule-based**: Deterministic pattern matching triggers specific handlers. No LLM reasoning is involved in tool selection. Examples: Discord slash commands, Slack bot commands.
- **Hybrid**: Text patterns or UI affordances narrow the candidate set, but the LLM makes the final selection or parameterization. Examples: GitHub Copilot Chat participants, Cursor agent mode with provider-level tool calling.

### Relevance to Context Manager

Context Manager's graph-based architecture raises specific questions about tool invocation:

- Should tools be registered as graph nodes with typed edges to their schemas?
- How should the system decide between LLM-driven and deterministic invocation?
- How do tool results flow back into the graph as new nodes?
- What scaling strategies exist when the tool count exceeds what fits in a context window?

Each system surveyed below offers partial answers to these questions.

---

## 2. Anthropic Claude tool_use

### 2.1 Tool Definition Schema

Tools are defined as JSON objects passed in the `tools` array of an API request. Each tool has a `name`, `description`, and `input_schema` (JSON Schema):

```json
{
  "name": "get_weather",
  "description": "Get the current weather in a given location.",
  "input_schema": {
    "type": "object",
    "properties": {
      "location": {
        "type": "string",
        "description": "City and state, e.g. San Francisco, CA"
      },
      "unit": {
        "type": "string",
        "enum": ["celsius", "fahrenheit"],
        "description": "Temperature unit"
      }
    },
    "required": ["location"]
  }
}
```

Key design decisions:

- Descriptions are critical for tool selection -- Anthropic's documentation emphasizes that the model relies heavily on description quality to decide when and how to use a tool.
- The `input_schema` uses standard JSON Schema, enabling validation before execution.
- Tool names must match `^[a-zA-Z0-9_-]{1,64}$`.

### 2.2 Invocation Lifecycle

The lifecycle follows a multi-turn request/response pattern:

```
User Message
    |
    v
API Request (messages + tools)
    |
    v
Claude reasons about which tool to call
    |
    v
Response with stop_reason="tool_use"
Contains: tool_use content block {id, name, input}
    |
    v
Client validates and executes the tool
    |
    v
Client sends tool_result message (role="user")
Contains: {type: "tool_result", tool_use_id, content}
    |
    v
Claude incorporates result and continues
Response with stop_reason="end_turn" (or another tool_use)
```

The critical architectural point: Claude does not execute tools itself. It emits a structured `tool_use` block, and the client is responsible for execution. The `tool_use_id` field links the result back to the specific invocation, enabling multiple tool calls in a single response.

Multiple tool calls can occur in a single response. Claude may return several `tool_use` blocks, and the client should execute all of them before sending all results back in a single message.

### 2.3 Server Tools vs Client Tools

Anthropic distinguishes between two categories:

- **Client tools**: Defined by the developer, executed by the client application. This is the standard `tools` array mechanism described above.
- **Server tools**: Executed by Anthropic's infrastructure. Examples include `web_search`, `code_execution`, and `text_editor`. These are enabled via a `server_tools` parameter and do not require client-side execution logic.

Server tools like `text_editor` provide built-in file manipulation capabilities (view, create, str_replace, insert, undo_edit commands) that Claude can invoke directly, with Anthropic handling execution in a sandboxed environment.

### 2.4 Advanced Features

**Tool Search (Deferred Loading)**

When the tool count is large (50+), loading all definitions into the system prompt creates two problems: token cost and accuracy degradation. Anthropic's internal testing showed 58 tools consuming approximately 55,000 tokens.

Tool search addresses this by deferring tool definitions. Tools are registered by name only, and full schemas are fetched on demand when the model determines a tool might be relevant. This can reduce token usage by up to 85% while maintaining selection accuracy.

In Claude Code's implementation, deferred tools appear in an `<available-deferred-tools>` block listing only names. A `ToolSearch` meta-tool allows the model to fetch full schemas via keyword search or exact name selection before invocation.

**Programmatic Tool Calling**

The `tool_choice` field controls how the model interacts with tools:

- `tool_choice: {"type": "tool", "name": "specific_tool"}` forces the model to call exactly that tool.
- `tool_choice: {"type": "auto"}` lets the model decide (default).
- `tool_choice: {"type": "any"}` forces the model to call at least one tool but lets it choose which.

**Streaming**

When streaming is enabled, tool calls arrive as incremental `content_block_delta` events of type `input_json_delta`. The client accumulates JSON fragments until the `content_block_stop` event, then parses and executes.

### 2.5 Claude Code: Tools in Practice

Claude Code (Anthropic's CLI agent) provides a concrete implementation of Claude tool_use at scale. Its system prompt defines 20+ tools using the standard JSON schema mechanism, but the UX layer adds several patterns:

- **Tool categories**: Tools are organized into groups (file operations, search, terminal, browser, MCP) with clear usage guidance in the system prompt.
- **Permission model**: Tools are classified by risk level. File reads are auto-approved; file writes and terminal commands require user confirmation (unless in permissive mode).
- **Hooks**: Shell commands that fire at specific lifecycle points (pre-tool, post-tool, notification) outside the LLM's reasoning chain. These enforce invariants that the model cannot override.
- **Sub-agents**: The `Task` tool spawns child agents for parallel research or implementation, each with their own tool access and conversation history.

### 2.6 Pros and Cons

| Aspect | Assessment |
|---|---|
| **Strengths** | Clean separation of concerns (model selects, client executes). Strong schema validation. Multi-tool responses. Server tools reduce client complexity. Tool search scales to large tool sets. |
| **Weaknesses** | Multi-turn overhead for simple tool calls. No native parallel execution guarantee (model may serialize). Tool descriptions require careful engineering. Token cost of tool definitions in every request. |

---

## 3. OpenAI Function Calling

### 3.1 Schema Definition

OpenAI supports tool calling through two APIs with slightly different schemas.

**Responses API** (newer, recommended):

```json
{
  "type": "function",
  "name": "get_weather",
  "description": "Get current weather for a location.",
  "parameters": {
    "type": "object",
    "properties": {
      "location": {"type": "string"},
      "unit": {"type": "string", "enum": ["c", "f"]}
    },
    "required": ["location"],
    "additionalProperties": false
  },
  "strict": true
}
```

**Chat Completions API** (legacy, widely used):

```json
{
  "type": "function",
  "function": {
    "name": "get_weather",
    "description": "Get current weather for a location.",
    "parameters": {
      "type": "object",
      "properties": {
        "location": {"type": "string"},
        "unit": {"type": "string", "enum": ["c", "f"]}
      },
      "required": ["location"],
      "additionalProperties": false
    },
    "strict": true
  }
}
```

The key difference is nesting: the Responses API uses flat top-level fields, while Chat Completions wraps everything inside a `function` key. This is a practical gotcha when migrating between APIs.

### 3.2 Invocation Lifecycle

**Responses API**:

```
Input: messages + tools
    |
    v
Model generates function_call output item
{type: "function_call", name, arguments, call_id}
    |
    v
Client executes the function
    |
    v
Client appends function_call_output item
{type: "function_call_output", call_id, output}
    |
    v
Model continues with next response
```

**Chat Completions API**:

```
Input: messages + tools
    |
    v
Response with finish_reason="tool_calls"
message.tool_calls = [{id, type: "function", function: {name, arguments}}]
    |
    v
Client executes each function
    |
    v
Client appends messages with role="tool"
{role: "tool", tool_call_id, content}
    |
    v
Model continues
```

Like Anthropic, OpenAI does not execute tools -- it returns structured invocation requests that the client handles.

### 3.3 Parallel Tool Calls

OpenAI models can return multiple tool calls in a single response. The model decides independently whether to parallelize. The `parallel_tool_calls` parameter (default `true`) controls whether the model is allowed to emit multiple simultaneous calls.

When enabled, the response may contain an array of tool calls. The client should execute all of them (potentially in parallel) and return all results before the next model turn.

Setting `parallel_tool_calls: false` forces sequential single-tool responses, which can be useful when tool calls have dependencies.

### 3.4 Strict Mode and Structured Outputs

Setting `strict: true` on a tool definition enables Structured Outputs, which guarantees the model's output will match the provided JSON Schema exactly. This eliminates the need for client-side argument validation and retry logic.

Constraints when using strict mode:

- All fields must be `required`.
- `additionalProperties` must be `false`.
- Some JSON Schema features are not supported (e.g., `oneOf` with more than 5 variants).
- The schema is compiled on first use, adding latency to the initial request.

### 3.5 Model Support Matrix

Not all OpenAI models support all tool calling features:

| Feature | gpt-4o | gpt-4o-mini | o1/o3 | gpt-3.5-turbo |
|---|---|---|---|---|
| Basic tool calling | Yes | Yes | Yes | Yes |
| Parallel tool calls | Yes | Yes | Limited | Yes |
| Strict mode | Yes | Yes | Yes | No |
| Streaming tool calls | Yes | Yes | No | Yes |

The reasoning models (o1, o3) have restrictions on streaming and parallel calls due to their chain-of-thought architecture.

### 3.6 Hosted Tools

Similar to Anthropic's server tools, OpenAI provides built-in tools that execute on their infrastructure:

- **Web search**: `{"type": "web_search_preview"}` -- model can search the web and cite sources.
- **File search**: For Assistants API, searches uploaded files using RAG.
- **Code interpreter**: Executes Python in a sandboxed environment.

These hosted tools reduce client implementation burden but limit customization.

### 3.7 Pros and Cons

| Aspect | Assessment |
|---|---|
| **Strengths** | Native parallel tool calls. Strict mode eliminates validation bugs. Two API surfaces for different use cases. Hosted tools for common operations. Wide model support. |
| **Weaknesses** | Schema differences between APIs cause migration friction. Reasoning models have limited tool support. `arguments` field is a JSON string (not parsed object) in Chat Completions, requiring client-side parsing. No equivalent to Anthropic's tool search for scaling. |

---

## 4. LangChain / LangGraph Agent Tool Invocation

### 4.1 Tool Definition

LangChain provides multiple ways to define tools.

**@tool decorator** (most common):

```python
from langchain_core.tools import tool

@tool
def multiply(a: int, b: int) -> int:
    """Multiply two numbers together.

    Args:
        a: First number
        b: Second number
    """
    return a * b
```

The decorator extracts the function name, docstring (as description), and type hints (as schema) automatically. The docstring is critical -- it becomes the description the LLM uses for tool selection.

**StructuredTool** (explicit schema):

```python
from langchain_core.tools import StructuredTool
from pydantic import BaseModel, Field

class SearchInput(BaseModel):
    query: str = Field(description="Search query string")
    max_results: int = Field(default=10, description="Maximum results")

search_tool = StructuredTool.from_function(
    func=search_web,
    name="web_search",
    description="Search the web for information",
    args_schema=SearchInput,
)
```

**BaseTool subclass** (maximum control):

```python
from langchain_core.tools import BaseTool

class DatabaseQueryTool(BaseTool):
    name: str = "query_database"
    description: str = "Execute a read-only SQL query"

    def _run(self, query: str) -> str:
        # execution logic
        ...

    async def _arun(self, query: str) -> str:
        # async execution logic
        ...
```

### 4.2 The ReAct Pattern

LangChain agents primarily use the ReAct (Reason + Act) pattern, which interleaves reasoning and tool use in a loop:

```
Observation: [initial context / user query]
    |
    v
Thought: I need to find X. Let me use the search tool.
    |
    v
Action: search_tool(query="X")
    |
    v
Observation: [search results]
    |
    v
Thought: Now I have X, I need to calculate Y.
    |
    v
Action: calculator(expression="...")
    |
    v
Observation: [result]
    |
    v
Thought: I now have enough information to answer.
    |
    v
Final Answer: [response to user]
```

The loop continues until the model decides it has sufficient information and emits a final answer instead of another tool call. This is fundamentally LLM-driven -- the model decides at each step whether to call a tool or terminate.

### 4.3 create_react_agent Architecture

LangGraph (the graph-based orchestration layer built on LangChain) implements ReAct as a stateful graph:

```
                    +-----------+
                    |  __start__|
                    +-----+-----+
                          |
                          v
                    +-----+-----+
              +---->|   agent    |<----+
              |     +-----+-----+     |
              |           |           |
              |     [conditional]     |
              |      /         \      |
              |     v           v     |
              | +---+---+   +--+--+  |
              | | tools  |   | end |  |
              | +---+---+   +-----+  |
              |     |                 |
              +-----+                 |
                                      |
         (loop until no tool calls)---+
```

The graph has two nodes:

- **agent node**: Calls the LLM with the current state (messages + tool schemas). If the LLM returns tool calls, execution routes to the tools node. If not, execution routes to end.
- **tools node**: Executes all requested tool calls and appends results to the message history. Then routes back to the agent node.

The conditional edge between agent and tools/end is the core decision point:

```python
from langgraph.prebuilt import create_react_agent

agent = create_react_agent(
    model=ChatAnthropic(model="claude-sonnet-4-20250514"),
    tools=[multiply, search_tool],
)

result = agent.invoke({"messages": [("user", "What is 6 * 7?")]})
```

### 4.4 Tool Binding

Before tools can be used, they must be bound to a model. This converts LangChain tool schemas into the provider-specific format:

```python
from langchain_anthropic import ChatAnthropic

model = ChatAnthropic(model="claude-sonnet-4-20250514")
model_with_tools = model.bind_tools([multiply, search_tool])

# The model now knows about these tools and can invoke them
response = model_with_tools.invoke("What is 6 * 7?")
# response.tool_calls = [{"name": "multiply", "args": {"a": 6, "b": 7}}]
```

`bind_tools` handles the translation from LangChain's universal tool schema to Anthropic's `tools` format, OpenAI's `tools` format, or Google's `function_declarations` format transparently.

### 4.5 Advanced Features

**Pre/Post Tool Hooks**

LangGraph supports hooks that execute before or after tool invocation:

```python
agent = create_react_agent(
    model=model,
    tools=tools,
    pre_tool_hook=lambda state: log_tool_call(state),
    post_tool_hook=lambda state: validate_result(state),
)
```

**Human-in-the-Loop**

LangGraph's `interrupt()` function pauses execution at any point and returns control to the human:

```python
from langgraph.types import interrupt

@tool
def dangerous_action(target: str) -> str:
    """Perform a dangerous action that requires approval."""
    approval = interrupt({"action": "dangerous_action", "target": target})
    if approval != "approved":
        return "Action cancelled by user."
    return execute_dangerous_action(target)
```

The graph state is persisted (via checkpointer), and execution resumes from the interrupt point when the human responds.

**Structured Output**

Tools can return structured data that gets validated against a schema before being passed back to the model, ensuring type safety throughout the pipeline.

### 4.6 Pros and Cons

| Aspect | Assessment |
|---|---|
| **Strengths** | Provider-agnostic tool binding. Graph-based orchestration enables complex workflows beyond simple loops. Built-in human-in-the-loop. State persistence and replay. Extensive ecosystem of pre-built tools. |
| **Weaknesses** | Abstraction overhead adds latency. Debugging multi-step agent runs is difficult. The `@tool` decorator magic can obscure what schema the model actually sees. Version churn between LangChain and LangGraph APIs. |

---

## 5. AI Code Editors

### 5.1 Cursor Agent Mode

Cursor is a VS Code fork that integrates LLM-driven coding as a first-class feature. Its Agent mode is the default and most autonomous mode.

**Tool Registration**

Cursor does not expose tool definitions in its system prompt in the way Cline does. Instead, it relies on the provider's native tool calling API (Anthropic, OpenAI, or Gemini). The tool schemas are injected into the API request by Cursor's backend, not visible to the user.

Available tools include:

- `edit_file` -- modify file contents
- `create_file` -- create new files
- `delete_file` -- remove files
- `run_terminal_command` -- execute shell commands
- `search_codebase` -- semantic search across the project
- `read_file` -- read file contents
- `list_directory` -- list directory contents
- `web_search` -- search the internet for documentation

**Dual-Model File Editing**

A distinctive architectural choice: the "main" frontier model (e.g., Claude Sonnet, GPT-4o) makes the `edit_file` tool call with a description of changes, and a weaker, faster model applies the actual diff. This optimizes cost and latency -- the expensive model reasons about what to change, while a cheap model handles the mechanical text transformation.

**Invocation Lifecycle**

```
User prompt in Composer
    |
    v
Cursor agent harness builds API request
(messages + tool definitions in provider-native format)
    |
    v
LLM returns tool_use / function_call
    |
    v
Cursor executes tool locally
(file I/O, terminal command, codebase search)
    |
    v
Result appended to conversation
    |
    v
LLM continues (loop until task complete)
    |
    v
Checkpoint created for rollback
```

**YOLO Mode**

When enabled, terminal commands and file deletions execute without confirmation. Useful for rapid prototyping in sandboxed environments, but dangerous in production codebases.

**Background Agents**

Cursor 2.0 introduced background agents that run in isolated Ubuntu VMs with internet access. They work on separate git branches, execute multi-step tasks asynchronously, and create PRs when done. Up to 8 agents can run in parallel using git worktree isolation.

**Agent Harness Tuning**

Cursor tunes its agent harness per model. Different frontier models respond differently to the same tool definitions -- a model trained on shell workflows might prefer `grep` over a dedicated search tool. Cursor's harness adjusts instructions and tool descriptions based on which model the user has selected.

### 5.2 Windsurf Cascade

Windsurf (formerly Codeium) uses its "Cascade" system as its agent mode.

**Architecture**

Cascade operates more autonomously than Cursor's agent by default. It can analyze code, run tests, and make changes without asking for permission at every step -- a design choice that prioritizes speed over control.

**SWE-grep Fast Context**

Windsurf's key technical innovation is SWE-grep, a set of models that retrieve relevant code context 10x faster than traditional agentic search. It achieves this by running 8 parallel tool calls per turn across only 4 turns, compared to the typical sequential search-read-search-read pattern.

**Validation Approach**

Windsurf escalates into log inspection more aggressively than other tools. When encountering failures, it inspects error states, isolates schema mismatches, adjusts token structures, and retests endpoints programmatically before concluding. It formalizes acceptance criteria into repeatable checks -- a backend-centric validation philosophy.

**Tool Execution**

Like Cursor, Windsurf uses provider-level tool calling APIs. The tools available are similar (file read/write, terminal, search, browser), but the orchestration differs in its autonomy level and parallel execution strategy.

### 5.3 Cline

Cline is an open-source VS Code extension that provides the most transparent view into AI code editor tool invocation.

**Tool Definition Format**

Cline defines tools directly in its system prompt using XML-style tags. The system prompt identifies over 20 tools with explicit usage descriptions:

```xml
<tool>
<name>read_file</name>
<description>
  Read the contents of a file at the specified path.
</description>
<parameters>
  <path>The path of the file to read (relative to workspace root)</path>
</parameters>
<usage>
<read_file>
<path>src/main.rs</path>
</read_file>
</usage>
</tool>
```

Available tools include: `execute_command`, `read_file`, `write_to_file`, `replace_in_file`, `search_files`, `list_files`, `list_code_definition_names`, `browser_action`, `use_mcp_tool`, `access_mcp_resource`, `ask_followup_question`, `attempt_completion`, `plan_mode_respond`.

**Tool-First Enforcement**

Cline's most distinctive design decision: the agent loop enforces that every model response must contain a tool call. If the model returns plain text without a tool invocation, the system rejects it:

```
[ERROR] You did not use a tool in your previous response!
Please retry with a tool use.
```

This includes conversational responses -- even "just talking" requires the `plan_mode_respond` or `ask_followup_question` tool. This guarantees structured, parseable output at every step of the agent loop.

**ReAct Loop**

Cline implements a ReAct (Reason-Act-Observe) loop:

1. **Reason**: The system prompt, conversation history, environment details, and tool definitions are sent to the LLM.
2. **Act**: The LLM returns a tool call (enforced). Cline parses the XML, validates parameters, and presents the action to the user for approval.
3. **Observe**: The tool result is appended to the conversation. The loop repeats.

After executing one tool, the agent immediately recurses with that result before considering the next tool. This prevents context bloat from accumulating multiple unprocessed results.

**Model-Adaptive Prompts**

Cline adapts its system prompt based on the model being used. Different model families receive different tool formats (XML vs. native JSON) and model-specific instructions. This enables Cline to work across the entire spectrum from frontier models to local LLMs running via Ollama.

**MCP Integration**

Cline natively supports MCP, allowing it to discover and invoke tools from external MCP servers. The `use_mcp_tool` and `access_mcp_resource` tools in Cline's system prompt act as bridges to the MCP ecosystem.

### 5.4 Comparison Table

| Feature | Cursor | Windsurf | Cline |
|---|---|---|---|
| Source | Proprietary (VS Code fork) | Proprietary (VS Code fork) | Open source (VS Code extension) |
| Tool format | Provider-native API | Provider-native API | XML in system prompt |
| Tool selection | LLM-driven via API | LLM-driven via API | LLM-driven, tool-first enforced |
| Permission model | Per-action approval (YOLO optional) | Mostly autonomous | Per-action approval (always) |
| Parallel execution | Up to 8 background agents | 8 parallel tool calls via SWE-grep | Sequential (one tool per turn) |
| File editing | Dual-model (frontier + fast) | Single model | Single model |
| MCP support | Yes | Yes | Yes (native) |
| Model flexibility | OpenAI, Anthropic, Gemini, xAI | Proprietary + select models | Any provider (including local) |

### 5.5 Pros and Cons

| System | Strengths | Weaknesses |
|---|---|---|
| **Cursor** | Fast dual-model editing. Background agents for async work. Per-model tuning. | Proprietary, opaque tool definitions. YOLO mode is risky. Cost scales with agent count. |
| **Windsurf** | Fast context retrieval (SWE-grep). Aggressive validation. Autonomous workflow. | Less user control. Proprietary. Limited model selection. |
| **Cline** | Fully transparent (open source). Tool-first enforcement ensures structured output. Works with any model. MCP native. | Sequential execution (no parallel tools). Approval fatigue from per-action confirmation. Token-heavy system prompt. |

---

## 6. Slash Commands and Inline Triggers

### 6.1 Discord Application Commands

Discord provides a formalized system for registering and invoking commands.

**Registration**

Commands are registered via the Discord API, either globally (available in all guilds, propagates in ~1 hour) or per-guild (available immediately):

```python
# Using discord.py
@bot.tree.command(name="weather", description="Get weather for a location")
@app_commands.describe(location="City name", unit="Temperature unit")
@app_commands.choices(unit=[
    app_commands.Choice(name="Celsius", value="c"),
    app_commands.Choice(name="Fahrenheit", value="f"),
])
async def weather(
    interaction: discord.Interaction,
    location: str,
    unit: str = "c",
):
    await interaction.response.defer()
    result = await fetch_weather(location, unit)
    await interaction.followup.send(result)
```

Command types include:

- **Slash commands** (`/command`): Text-based commands with typed parameters.
- **User commands**: Right-click on a user to trigger.
- **Message commands**: Right-click on a message to trigger.

**Invocation Lifecycle**

```
User types /weather
    |
    v
Discord client shows autocomplete with registered parameters
    |
    v
User submits command
    |
    v
Discord sends Interaction webhook (HTTP POST) to bot
{type: 2, data: {name: "weather", options: [{name: "location", value: "NYC"}]}}
    |
    v
Bot must acknowledge within 3 seconds (ack or defer)
    |
    v
Bot executes logic and sends response
(followup message, ephemeral or public)
    |
    v
Response appears in channel
```

The 3-second acknowledgment deadline is a hard constraint. Long-running operations must use `defer()` to signal that a response is coming, then send a followup when ready.

**Parameter Types**

Discord supports typed parameters with validation: `STRING`, `INTEGER`, `BOOLEAN`, `USER`, `CHANNEL`, `ROLE`, `MENTIONABLE`, `NUMBER`, `ATTACHMENT`. Autocomplete callbacks can dynamically suggest values as the user types.

### 6.2 Slack Slash Commands and Bolt Framework

**Registration**

Slash commands are registered in the Slack App configuration (via UI or manifest). Each command specifies a request URL that Slack will POST to.

**Bolt Framework Pattern**

The Bolt SDK (available in JS, Python, Java) provides a clean listener pattern:

```python
from slack_bolt import App

app = App(token="xoxb-...", signing_secret="...")

@app.command("/deploy")
def handle_deploy(ack, respond, command):
    ack()  # Must acknowledge within 3 seconds
    environment = command["text"]  # Raw text after /deploy
    result = deploy_to(environment)
    respond(f"Deployed to {environment}: {result}")
```

**Invocation Lifecycle**

```
User types /deploy production
    |
    v
Slack sends HTTP POST to configured URL
{command: "/deploy", text: "production", user_id: "U123", channel_id: "C456"}
    |
    v
Handler calls ack() within 3 seconds
    |
    v
Handler executes logic
    |
    v
Handler calls respond() to reply in channel
(or uses response_url for delayed responses up to 30 minutes)
```

**Key Differences from Discord**

- Slack slash commands receive raw text (no typed parameters). Parsing is the handler's responsibility.
- No built-in autocomplete for parameters.
- Interactive components (buttons, select menus, modals) provide richer parameter input but are separate from the slash command itself.
- Block Kit provides a UI framework for structured responses.

**Workflow Steps**

Slack's Workflow Builder can incorporate custom steps backed by apps, enabling no-code composition of slash command-like actions into multi-step workflows.

### 6.3 GitHub Copilot Chat

GitHub Copilot Chat uses three types of inline triggers, each serving a different purpose.

**Slash Commands (`/`)**

Built-in commands that trigger specific behaviors:

- `/explain` -- explain selected code
- `/fix` -- suggest a fix for problems
- `/tests` -- generate unit tests
- `/doc` -- generate documentation
- `/new` -- scaffold a new project
- `/clear` -- clear the chat session

These are deterministic triggers -- typing `/fix` always invokes the fix behavior, regardless of LLM reasoning.

**Chat Participants (`@`)**

Participants are domain experts that scope the conversation:

- `@workspace` -- context about the entire workspace
- `@vscode` -- VS Code editor commands and settings
- `@terminal` -- terminal context and commands
- `@github` -- GitHub-specific operations (issues, PRs, repos)

Participants narrow the tool set and context available to the LLM. When you type `@workspace`, the model gets access to workspace search tools that it would not have in a generic chat.

**Context Variables (`#`)**

Variables inject specific context into the prompt:

- `#file` -- reference a specific file
- `#selection` -- the current editor selection
- `#editor` -- visible content in the active editor
- `#terminalLastCommand` -- last terminal command and output
- `#codebase` -- broader codebase context

These are purely additive -- they do not trigger actions but enrich the context available to the LLM.

**Agent Mode and MCP**

GitHub Copilot's Agent Mode (introduced in VS Code) enables autonomous multi-step workflows similar to Cursor and Cline. It can edit files, run terminal commands, and iterate. Crucially, it supports MCP servers, allowing users to connect external tools (databases, APIs, deployment systems) that the agent can invoke.

### 6.4 Pattern Taxonomy

| Pattern | Trigger | Selection | Execution | Examples |
|---|---|---|---|---|
| Slash command | `/command` | Rule-based (exact match) | Deterministic handler | Discord, Slack, Copilot Chat |
| Mention/Participant | `@name` | Rule-based (scoping) | Context narrowing | Copilot Chat, Slack apps |
| Context variable | `#reference` | Rule-based (injection) | Context enrichment | Copilot Chat |
| Natural language | Free text | LLM-driven | Model selects tool | All agent systems |
| Hybrid | `@scope` + free text | Rule-based scope + LLM selection | Scoped LLM reasoning | Copilot Chat participants |

### 6.5 Pros and Cons

| Aspect | Assessment |
|---|---|
| **Strengths** | Slash commands are discoverable (autocomplete), predictable (deterministic), and fast (no LLM reasoning needed). Low latency. Clear user intent. Easy to document and learn. |
| **Weaknesses** | Rigid -- cannot handle ambiguous or novel requests. Parameter parsing is manual in Slack. Combinatorial explosion as command count grows. Cannot compose commands without workflow systems. |

---

## 7. Model Context Protocol (MCP)

### 7.1 Architecture

MCP defines a client-server protocol for connecting AI models to external tools and data sources. The architecture has three layers:

- **Host**: The application (IDE, chat interface, agent framework) that manages one or more MCP clients.
- **Client**: Maintains a 1:1 connection with an MCP server. Handles protocol negotiation, capability exchange, and message routing.
- **Server**: Exposes tools, resources, and prompts to clients via a standardized interface.

```
+-------+     +--------+     +----------+
| Host  |---->| Client |---->| Server A |  (filesystem tools)
|       |     +--------+     +----------+
|       |
|       |     +--------+     +----------+
|       |---->| Client |---->| Server B |  (database tools)
|       |     +--------+     +----------+
+-------+
```

Each server is an independent process. The host aggregates tools from all connected servers and presents them to the LLM as a unified tool set.

### 7.2 Tool Schema

Tools are defined using JSON Schema for inputs and (optionally) outputs:

```json
{
  "name": "query_database",
  "description": "Execute a read-only SQL query against the analytics database.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "query": {
        "type": "string",
        "description": "SQL SELECT query to execute"
      },
      "database": {
        "type": "string",
        "enum": ["analytics", "warehouse"],
        "description": "Target database"
      }
    },
    "required": ["query"]
  },
  "outputSchema": {
    "type": "object",
    "properties": {
      "rows": {
        "type": "array",
        "items": {"type": "object"}
      },
      "row_count": {"type": "integer"}
    }
  }
}
```

The `outputSchema` field (added in later MCP revisions) enables the host to validate server responses and provides the LLM with expectations about result structure.

MCP also defines two other capability types beyond tools:

- **Resources**: Read-only data sources (files, database records, API responses) that the client can fetch. Identified by URI.
- **Prompts**: Reusable prompt templates that servers can expose for common operations.

### 7.3 Invocation Lifecycle

MCP uses JSON-RPC 2.0 as its wire protocol:

```
1. Discovery: Client sends tools/list request
   --------->
   Server responds with array of tool definitions
   <---------

2. LLM reasons and selects a tool (outside MCP)

3. Invocation: Client sends tools/call request
   {
     "jsonrpc": "2.0",
     "method": "tools/call",
     "params": {
       "name": "query_database",
       "arguments": {"query": "SELECT count(*) FROM events"}
     },
     "id": 1
   }
   --------->

4. Execution: Server validates, executes, returns result
   {
     "jsonrpc": "2.0",
     "result": {
       "content": [
         {"type": "text", "text": "Query returned 42,891 rows"}
       ]
     },
     "id": 1
   }
   <---------

5. Host appends result to conversation context
6. LLM incorporates result and continues
```

MCP itself does not decide which tool to call -- that decision is made by the LLM (or host application logic) outside the protocol. MCP is purely the transport and schema layer.

**Notifications**

MCP supports server-initiated notifications:

- `notifications/tools/list_changed` -- server signals that its tool set has changed (tools added, removed, or modified). The client should re-fetch `tools/list`.
- Progress notifications for long-running tool executions.

### 7.4 Transport Layer

MCP separates the data layer (JSON-RPC messages) from the transport layer. Three transports are defined:

**stdio**

The server runs as a child process. Communication happens over stdin/stdout. This is the simplest transport, used by most local MCP servers (filesystem, git, database tools).

```
Host spawns server process
    |
    stdin: JSON-RPC requests --->
    stdout: <--- JSON-RPC responses
    stderr: <--- logging (ignored by protocol)
```

**HTTP + Server-Sent Events (SSE)**

For remote servers. The client connects via HTTP:

- Client-to-server: HTTP POST requests with JSON-RPC bodies.
- Server-to-client: SSE stream for responses and notifications.

**Streamable HTTP**

A newer transport that uses a single HTTP endpoint supporting both request/response and streaming patterns. Designed to work better with modern infrastructure (load balancers, proxies, serverless).

### 7.5 Security Considerations

MCP's security model places responsibility on the host:

- **No built-in authentication**: The protocol does not define auth. Hosts must implement their own auth layer (API keys, OAuth, mTLS) depending on transport.
- **Tool approval**: The host should present tool calls to the user for approval before execution, especially for tools with side effects.
- **Input validation**: Servers should validate all inputs against their `inputSchema` before execution, even though the schema is also available to clients for pre-validation.
- **Sandboxing**: stdio servers run as local processes with the host's permissions. Remote servers (HTTP/SSE) have their own security boundary.

The specification recommends that hosts implement a consent model where users explicitly approve which MCP servers to connect and which tools to allow.

### 7.6 Pros and Cons

| Aspect | Assessment |
|---|---|
| **Strengths** | Protocol-level standardization enables tool portability across hosts. Clean separation of concerns (host reasons, server executes). Transport-agnostic design. Dynamic tool discovery via `tools/list`. Growing ecosystem (Anthropic, OpenAI, Google, Microsoft all support MCP). |
| **Weaknesses** | No built-in security or authentication. No native tool composition (cannot chain tools at the protocol level). Overhead of running separate server processes. Schema validation is optional, not enforced. Version negotiation adds complexity. |

---

## 8. Cross-Cutting Analysis

### 8.1 Comparison Matrix

| Dimension | Claude tool_use | OpenAI function calling | LangChain/LangGraph | Cursor/Windsurf | Cline | Slash commands | MCP |
|---|---|---|---|---|---|---|---|
| **Schema format** | JSON Schema | JSON Schema | Python types + docstrings | Provider-native | XML in prompt | Platform-specific | JSON Schema (JSON-RPC) |
| **Selection mechanism** | LLM-driven | LLM-driven | LLM-driven (ReAct) | LLM-driven | LLM-driven (enforced) | Rule-based | Host-dependent |
| **Parallel calls** | Yes (multi-block) | Yes (native) | Yes (via graph) | Yes (background agents) | No (sequential) | N/A | No (per-call) |
| **Result feedback** | tool_result message | tool role message | State accumulation | Conversation append | Conversation append | Channel message | JSON-RPC response |
| **Human-in-the-loop** | Client-implemented | Client-implemented | Built-in (interrupt) | Per-action / YOLO | Always (per-action) | Implicit (user initiates) | Host-implemented |
| **Scaling strategy** | Tool search (deferred) | None built-in | Tool filtering | Per-model tuning | Model-adaptive prompts | Command registry | Dynamic discovery |
| **Execution location** | Client (or server tools) | Client (or hosted tools) | Client | Local (or VM) | Local | Bot server | MCP server |

### 8.2 Selection Strategy: When to Use What

**Use LLM-driven selection when:**

- The action space is large and varied.
- User intent is ambiguous or requires reasoning to map to tools.
- Tool parameters need to be extracted from natural language.
- The system needs to compose multiple tools to fulfill a request.

**Use rule-based selection when:**

- Actions are well-defined and finite.
- Latency matters (no LLM round-trip for selection).
- Deterministic behavior is required (same input always triggers same action).
- The user base expects predictable, discoverable commands.

**Use hybrid selection when:**

- A scoping mechanism (participant, context) narrows the tool set.
- The LLM should reason within a constrained domain.
- You want discoverability of slash commands with the flexibility of natural language.

### 8.3 Common Patterns

**Schema-First Design**

Every system surveyed uses some form of schema to describe tools -- JSON Schema (Claude, OpenAI, MCP), Python type hints (LangChain), XML descriptions (Cline), or platform-specific formats (Discord, Slack). The schema serves dual purposes: guiding LLM tool selection and enabling input validation.

**Feedback Loops**

All LLM-driven systems implement a feedback loop where tool results are injected back into the conversation as new messages. This is architecturally identical across systems -- the result becomes part of the context for the next reasoning step.

**Acknowledgment Deadlines**

Both Discord and Slack enforce 3-second acknowledgment deadlines, forcing async patterns for long-running operations. Agent systems handle this differently -- they are inherently asynchronous (the user waits for the agent loop to complete).

**Human-in-the-Loop as Spectrum**

Systems range from no approval (YOLO mode, slash commands initiated by user) to per-action approval (Cline) to selective approval (Cursor, with risk-based classification). LangGraph's `interrupt()` is the most flexible, allowing programmatic insertion of approval gates at any point.

### 8.4 Scaling Challenges

**Token Cost**

Tool definitions consume input tokens on every request. With 50+ tools, this can exceed 50,000 tokens per request. Mitigation strategies:

- **Deferred loading** (Anthropic tool search): Load schemas on demand. Reduces token use by up to 85%.
- **Dynamic filtering** (LangChain): Only bind relevant tools based on conversation context.
- **Model-specific tuning** (Cursor): Adjust tool descriptions per model to minimize tokens while maintaining accuracy.

**Accuracy Degradation**

As tool count increases, the LLM's ability to select the correct tool decreases. Research and practitioner reports suggest accuracy drops noticeably beyond 15-20 tools.

Mitigation strategies:

- **Hierarchical organization**: Group tools into categories; let the model first select a category, then a specific tool.
- **Tool descriptions**: Invest heavily in description quality -- clear, concise descriptions with usage examples improve selection accuracy more than any structural change.
- **Forced tool choice**: When the correct tool is known programmatically, use `tool_choice` to bypass LLM selection entirely.

---

## 9. Implications for Context Manager

### 9.1 Graph-Native Tool Registration

Context Manager's graph architecture suggests registering tools as first-class graph nodes:

```
[Tool: read_file] --schema--> [Schema: ReadFileInput]
[Tool: read_file] --category--> [Category: FileOps]
[Tool: read_file] --used_by--> [Task: Analyze codebase]
[Tool: read_file] --last_result--> [Result: {content: "..."}]
```

This enables:

- Querying tool usage history via graph traversal.
- Discovering related tools via category edges.
- Tracking which tasks use which tools for optimization.
- Pruning tool sets based on task context (subgraph extraction).

### 9.2 Hybrid Invocation Strategy

Based on the survey, Context Manager should adopt a hybrid approach:

1. **Deterministic triggers** for well-known operations: `/focus`, `/summarize`, `/checkpoint` -- these map directly to graph operations and do not need LLM reasoning.
2. **LLM-driven selection** for open-ended tasks: "help me understand this module" should let the model choose between reading files, searching code, and querying the graph.
3. **Context-scoped selection**: When the user is focused on a specific graph node (a file, a task, a work item), narrow the available tool set to relevant operations.

### 9.3 Tool Result Integration

Tool results should flow back into the graph as new nodes:

- `ToolResult` node with edges to the tool that produced it, the task that requested it, and the conversation turn.
- Results are available for future context retrieval without re-execution.
- Graph compaction can summarize old tool results to save tokens.

### 9.4 Scaling via Deferred Loading

Following Anthropic's tool search pattern, Context Manager should:

- Register tools by name and category in the graph.
- Only inject full schemas for tools relevant to the current subgraph context.
- Use the graph's neighborhood structure to determine relevance.

---

## References

- Anthropic (2025) - [Tool Use Documentation](https://docs.anthropic.com/en/docs/build-with-claude/tool-use/overview)
- Anthropic (2025) - [Claude Code Agent Hooks and Skills](https://www.dotzlaw.com/insights/claude-deterministic-agent-engineering/)
- Anthropic (2025) - [Model Context Protocol Specification](https://modelcontextprotocol.io/)
- OpenAI (2025) - [Function Calling Guide](https://platform.openai.com/docs/guides/function-calling)
- OpenAI (2025) - [Responses API Tools](https://platform.openai.com/docs/api-reference/responses)
- LangChain (2025) - [Tool Calling Documentation](https://python.langchain.com/docs/concepts/tool_calling/)
- LangGraph (2025) - [create_react_agent](https://langchain-ai.github.io/langgraph/agents/overview/)
- Cursor (2025) - [Agent Mode Overview](https://docs.cursor.com/chat/agent)
- Cursor (2025) - [Agent System Prompt Analysis](https://gist.github.com/sshh12/25ad2e40529b269a88b80e7cf1c38084)
- Cursor (2026) - [Agent-First Architecture Guide](https://www.digitalapplied.com/blog/cursor-2-0-agent-first-architecture-guide)
- Cursor (2025) - [Agent Best Practices](https://cursor.com/blog/agent-best-practices)
- Windsurf (2025) - [Windsurf vs Cursor Comparison](https://windsurf.com/compare/windsurf-vs-cursor)
- Cline (2025) - [System Prompt Fundamentals](https://cline.bot/blog/system-prompt)
- Cline (2025) - [System Prompt Advanced](https://cline.bot/blog/system-prompt-advanced)
- Cline (2025) - [Open Source Repository](https://github.com/cline/cline)
- Flora Lan (2026) - [Inside Cline: How Its Agentic Chat System Really Works](https://medium.com/@floralan212/inside-cline-how-its-agentic-chat-system-really-works-3d582935efa5)
- Composio (2026) - [Tool Calling Explained: The Core of AI Agents](https://composio.dev/content/ai-agent-tool-calling-guide)
- Martin Fowler (2025) - [Context Engineering for Coding Agents](https://martinfowler.com/articles/exploring-gen-ai/context-engineering-coding-agents.html)
- x1xhlol (2025) - [System Prompts and Models of AI Tools](https://github.com/x1xhlol/system-prompts-and-models-of-ai-tools)
- Discord (2025) - [Application Commands Documentation](https://discord.com/developers/docs/interactions/application-commands)
- Slack (2025) - [Bolt Framework Documentation](https://slack.dev/bolt-python/)
- GitHub (2025) - [Copilot Chat Documentation](https://docs.github.com/en/copilot/using-github-copilot/using-github-copilot-chat)
- Vercel (2026) - [How We Built AEO Tracking for Coding Agents](https://vercel.com/blog/how-we-built-aeo-tracking-for-coding-agents)
- DataLakehouse Hub (2026) - [Context Management Strategies for VS Code with LLM Plugins](https://datalakehousehub.com/blog/2026-03-context-management-vscode-llm-plugins/)
- Skywork AI (2025) - [Cursor AI Review: Agent Mode, Repo-Wide Refactors](https://skywork.ai/blog/cursor-ai-review-2025-agent-refactors-privacy/)
