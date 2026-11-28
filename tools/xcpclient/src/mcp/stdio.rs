//-----------------------------------------------------------------------------
// Module mcp::stdio
// Protection of stdout for the MCP stdio transport
//
// On the stdio transport, stdout is the protocol channel: one line which is not an MCP message breaks the connection.
// xcpclient and the libraries it uses print to stdout in many places (log messages, the verbose output of the ELF reader).
// Instead of changing all of them, the MCP mode redirects stdout for the whole process: the MCP transport gets a private
// duplicate of the original stdout, and file descriptor 1 is redirected to stderr. Everything printed to stdout from then on
// arrives on stderr, where MCP hosts expect the log output of a server.

/// The output channel for the MCP messages
pub type McpOutput = Box<dyn tokio::io::AsyncWrite + Send + Unpin>;

/// Redirect stdout to stderr and return the original stdout as output channel for the MCP messages
/// Call it once and early, before anything is printed. Needs a tokio runtime
#[cfg(unix)]
pub fn protect_stdout() -> std::io::Result<McpOutput> {
    use std::io::Write;
    use std::os::fd::FromRawFd;

    std::io::stdout().flush()?;
    // SAFETY: dup and dup2 are called with the valid file descriptors 1 and 2 of the process, the duplicate is a new file
    // descriptor which is owned by the File from here on and by nothing else
    let original_stdout = unsafe {
        let fd = libc::dup(libc::STDOUT_FILENO);
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        if libc::dup2(libc::STDERR_FILENO, libc::STDOUT_FILENO) < 0 {
            let e = std::io::Error::last_os_error();
            libc::close(fd);
            return Err(e);
        }
        std::fs::File::from_raw_fd(fd)
    };
    Ok(Box::new(tokio::fs::File::from_std(original_stdout)))
}

/// Without redirection on this platform: stdout is the output channel for the MCP messages, nothing else may print to it
#[cfg(not(unix))]
pub fn protect_stdout() -> std::io::Result<McpOutput> {
    Ok(Box::new(tokio::io::stdout()))
}
