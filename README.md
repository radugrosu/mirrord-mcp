# Mirrord MCP Server MVP

## Overview

This project is a Minimum Viable Product (MVP) demonstrating a server that uses the Model Communication Protocol (MCP) to allow an AI assistant to test the changes it adds to an existing project against a specified Kubernetes deployment using `mirrord`.

It leverages the `rmcp` library for the MCP transport layer (specifically Server-Sent Events - SSE) and the `mirrord` CLI tool to inject the execution context into the target Kubernetes pod.

## Goal

The primary goal is to allow developers (or potentially automated tools) to quickly test small code snippets that interact with services within a Kubernetes cluster, without needing to manually set up `mirrord` or build/deploy full applications for simple tests.

## Features

* **MCP Server:** Implements an MCP server using `rmcp`.
* **Mirrord Integration:** Automatically configures and invokes `mirrord exec` to run a command string with in the context of a target Kubernetes deployment/pod.
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
  "tool_name": "run",
  "arguments": {
    "cmd_str": "node /abs/path/to/script.js",
    "deployment": "user-service",
    "mirrord_config": "{\"feature\": {\"network\": {\"incoming\": {\"mode\": \"mirror\", \"ports\": [8080]}}}}"
  }
}
```
