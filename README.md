# Mirrord MCP Server MVP

This project is a Minimum Viable Product (MVP) demonstrating a server that uses the Model Communication Protocol (MCP) to receive code snippets (Rust, Node.js, Python), execute them using `mirrord` against a specified Kubernetes deployment, and return the results.

It leverages the `rmcp` library for the MCP transport layer (specifically Server-Sent Events - SSE) and the `mirrord` CLI tool to inject the execution context into the target Kubernetes pod.

## Goal

The primary goal is to allow developers (or potentially automated tools) to quickly test small code snippets that interact with services within a Kubernetes cluster, without needing to manually set up `mirrord` or build/deploy full applications for simple tests.

## Features

* **MCP Server:** Implements an MCP server using `rmcp`.
* **Multi-Language Support:** Provides tools to execute:
  * Rust code snippets (compiles a temporary binary).
  * Node.js scripts (uses `npm` for dependencies).
  * Python scripts (uses `venv` and `pip` for dependencies).
* **Mirrord Integration:** Automatically configures and invokes `mirrord exec` to run the code snippet in the context of a target Kubernetes deployment/pod.
* **Dynamic Setup:** Creates temporary project directories, installs necessary dependencies (via `cargo`, `npm`, `pip`), and cleans up resources afterwards.
* **Asynchronous Processing:** Built with Tokio for non-blocking I/O.
* **Command Timeouts:** Implements timeouts for potentially long-running external commands (`kubectl`, build tools, `mirrord exec`) to prevent hangs.
* **Graceful Shutdown:** Handles `Ctrl+C` for clean termination.

## Technology Stack

* Rust
* Tokio (Asynchronous Runtime)
* `rmcp` (MCP Transport Library)
* `mirrord` (CLI Tool)
* `kubectl` (Kubernetes CLI)
* Serde (Serialization/Deserialization)
* Tracing (Logging)
* Tempfile (Temporary file/directory management)

## Prerequisites

Before running the server, ensure you have the following installed and configured:

1. **Rust Toolchain:** Install via rustup (`cargo`).
2. **`mirrord` CLI:** Install the latest version from the mirrord documentation.
3. **`kubectl`:** Install and configure it to connect to your target Kubernetes cluster. The server needs permission to get pods within the specified namespace.
4. **Node.js & `npm`:** Required if you intend to use the `run_node` tool.
5. **Python 3 & `pip`:** Required if you intend to use the `run_python` tool. Ensure `python3` is in your PATH.

## Setup & Running

1. **Clone the repository:**

    ```bash
    git clone <your-repo-url>
    cd mirrord-mcp-server
    ```

2. **Build the server:**

    ```bash
    cargo build --release
    ```

3. **Run the server:**

    ```bash
    ./target/release/mirrord-mcp-server
    ```

By default, the server listens on `127.0.0.1:3000` and exposes the SSE endpoint at `/sse`.

## Usage

This server is designed to be used with an MCP client. The client connects to the SSE endpoint (`http://127.0.0.1:3000/sse`) and sends `call_tool` requests.

**Example Client Interaction (Conceptual):**

A client would send a JSON message similar to this (structure depends on the specific tool):

```json
{
  "tool_name": "run_rust",
  "arguments": {
    "code": "use reqwest::blocking::get;\nuse serde::Deserialize;\nuse anyhow::Result;\n\n#[derive(Deserialize)]\nstruct Location {\n    country: String,\n    city: String,\n}\n\nfn main() -> Result<()> {\n    let url = \"http://localhost:8080/location\";\n    let resp = get(url)?.json::<Location>()?;\n    println!(\"Country: {}, City: {}\", resp.country, resp.city);\n    Ok(())\n}\n",
    "deployment": "user-service",
    "mirrord_config": "{\"feature\": {\"network\": {\"incoming\": {\"mode\": \"mirror\", \"ports\": [8080]}}}}"
  }
}
```

This message will normally composed by an llm agent running behind the client, i.e. the prompt for the message above would be something like:

> Get the user info - country and city from the user-service deployment running at /location on port 8080; use the node tool

## Security Considerations - IMPORTANT

Arbitrary Code Execution: This server executes code provided by the client. This is inherently dangerous. Only run this server in a trusted environment and only allow trusted clients to connect.
No Authentication/Authorization: The current MVP does not implement any authentication or authorization. Anyone who can reach the server endpoint can execute code. Do not expose this server to untrusted networks.
Resource Limits: No limits are placed on the CPU, memory, or execution time consumed by the build processes or the user code running via mirrord. A malicious or poorly written snippet could potentially cause a Denial-of-Service (DoS) attack on the server or the Kubernetes node where mirrord runs the code.
Permissions: The server process requires permissions to run kubectl, mirrord, cargo, npm, pip, and write to temporary directories. The mirrord execution itself inherits permissions based on its configuration and the target pod's service account (unless overridden).
This MVP is NOT suitable for production or untrusted environments without significant security hardening.

## Future Improvements

* Security: Implement robust authentication and authorization.
* Sandboxing: Explore sandboxing techniques (e.g., containers, gVisor, syscall filtering) to limit the impact of executed code.
* Resource Limits: Add configurable limits for CPU, memory, and execution time.
* Configuration: Allow configuration of the listening address, port, default namespace, and command timeouts via environment variables or a config file.
* Kubernetes Client Library: Replace kubectl command execution with a proper Rust Kubernetes client library (e.g., kube-rs) for more robust interaction.
* Streaming Output: Modify mirrord execution to stream stdout/stderr back instead of waiting for completion (would require changes to rmcp tool definition or custom handling).
* Error Handling: Provide more structured error types back to the client.
