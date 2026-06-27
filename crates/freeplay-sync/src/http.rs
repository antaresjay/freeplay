//! winhttp, not a http crate. freeplay moves a few small text files and
//! pulling in a tls stack and a certificate bundle for that would be most of
//! the dependency tree. winhttp is already there and uses the certificate
//! store windows already has, so proxies and pinned roots just work

#[cfg(windows)]
pub fn get(url: &str) -> Result<Vec<u8>, String> {
    send(url, "GET", None)
}

#[cfg(windows)]
pub fn post(url: &str, body: &[u8]) -> Result<Vec<u8>, String> {
    send(url, "POST", Some(body))
}

#[cfg(windows)]
fn send(url: &str, verb: &str, body: Option<&[u8]>) -> Result<Vec<u8>, String> {
    use windows::core::PCWSTR;
    use windows::Win32::Networking::WinHttp::*;

    let (host, path) = split(url)?;

    // nul terminated utf16, every one of these takes a PCWSTR
    let wide = |text: &str| -> Vec<u16> { text.encode_utf16().chain(std::iter::once(0)).collect() };
    let agent = wide("Freeplay");
    let host_w = wide(&host);
    let path_w = wide(&path);
    let verb_w = wide(verb);

    unsafe {
        let session = WinHttpOpen(
            PCWSTR(agent.as_ptr()),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        );
        if session.is_null() {
            return Err("could not start a http session".into());
        }
        let session = Handle(session);

        let connection = WinHttpConnect(session.0, PCWSTR(host_w.as_ptr()), 443, 0);
        if connection.is_null() {
            return Err(format!("could not reach {host}"));
        }
        let connection = Handle(connection);

        let request = WinHttpOpenRequest(
            connection.0,
            PCWSTR(verb_w.as_ptr()),
            PCWSTR(path_w.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null_mut(),
            WINHTTP_FLAG_SECURE,
        );
        if request.is_null() {
            return Err("could not build the request".into());
        }
        let request = Handle(request);

        let headers: Vec<u16> = "Content-Type: application/json\r\n"
            .encode_utf16()
            .collect();
        let (header_slice, payload, length) = match body {
            Some(data) => (
                Some(headers.as_slice()),
                Some(data.as_ptr() as *const core::ffi::c_void),
                data.len() as u32,
            ),
            None => (None, None, 0),
        };

        WinHttpSendRequest(request.0, header_slice, payload, length, length, 0)
            .map_err(|e| format!("could not send the request: {e}"))?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut())
            .map_err(|e| format!("no answer from {host}: {e}"))?;

        let status = status_code(request.0)?;
        if !(200..300).contains(&status) {
            let text = String::from_utf8_lossy(&read_body(request.0)?).to_string();
            return Err(match reason(&text) {
                Some(why) => why,
                None => format!("{host}{path} answered {status}"),
            });
        }

        read_body(request.0)
    }
}

#[cfg(windows)]
unsafe fn read_body(request: *mut core::ffi::c_void) -> Result<Vec<u8>, String> {
    use windows::Win32::Networking::WinHttp::*;

    unsafe {
        let mut body = Vec::new();
        loop {
            let mut available: u32 = 0;
            WinHttpQueryDataAvailable(request, &mut available)
                .map_err(|e| format!("read failed: {e}"))?;
            if available == 0 {
                break;
            }

            // a table that big is not a table
            if body.len() + available as usize > 8 * 1024 * 1024 {
                return Err("that response is far too large to be a table".into());
            }

            let mut chunk = vec![0u8; available as usize];
            let mut read: u32 = 0;
            WinHttpReadData(request, chunk.as_mut_ptr() as *mut _, available, &mut read)
                .map_err(|e| format!("read failed: {e}"))?;
            chunk.truncate(read as usize);
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }
}

#[cfg(windows)]
struct Handle(*mut core::ffi::c_void);

#[cfg(windows)]
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Networking::WinHttp::WinHttpCloseHandle(self.0);
        }
    }
}

#[cfg(windows)]
unsafe fn status_code(request: *mut core::ffi::c_void) -> Result<u32, String> {
    use windows::Win32::Networking::WinHttp::*;

    let mut code: u32 = 0;
    let mut size = std::mem::size_of::<u32>() as u32;
    unsafe {
        WinHttpQueryHeaders(
            request,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            windows::core::PCWSTR::null(),
            Some(&mut code as *mut u32 as *mut _),
            &mut size,
            std::ptr::null_mut(),
        )
        .map_err(|e| format!("could not read the status: {e}"))?;
    }
    Ok(code)
}

// the worker answers a failure with {"error":"..."} and that sentence is far
// more use than the status code on its own
pub fn reason(text: &str) -> Option<String> {
    let at = text.find("\"error\"")?;
    let rest = &text[at + 7..];
    let open = rest.find('"')? + 1;
    let close = rest[open..].find('"')? + open;
    Some(rest[open..close].to_string())
}

// only https, and only the host and path, because that is all this needs
pub fn split(url: &str) -> Result<(String, String), String> {
    let rest = url
        .strip_prefix("https://")
        .ok_or_else(|| format!("{url} is not https"))?;

    let (host, path) = match rest.find('/') {
        Some(at) => (&rest[..at], &rest[at..]),
        None => (rest, "/"),
    };

    if host.is_empty() || host.contains('@') {
        return Err(format!("{url} has no host worth trusting"));
    }
    Ok((host.to_string(), path.to_string()))
}

#[cfg(not(windows))]
pub fn get(_url: &str) -> Result<Vec<u8>, String> {
    Err("fetching is only implemented on windows so far".into())
}

#[cfg(not(windows))]
pub fn post(_url: &str, _body: &[u8]) -> Result<Vec<u8>, String> {
    Err("posting is only implemented on windows so far".into())
}

#[cfg(test)]
mod tests {
    use super::split;

    #[test]
    fn splits_a_url() {
        assert_eq!(
            split("https://example.com/a/b.json").unwrap(),
            ("example.com".into(), "/a/b.json".into())
        );
        assert_eq!(
            split("https://example.com").unwrap(),
            ("example.com".into(), "/".into())
        );
    }

    #[test]
    fn refuses_anything_but_https() {
        assert!(split("http://example.com/x").is_err());
        assert!(split("file:///etc/passwd").is_err());
        assert!(split("ftp://example.com").is_err());
    }

    // user@host in a url is the oldest trick there is
    #[test]
    fn refuses_a_userinfo_host() {
        assert!(split("https://raw.githubusercontent.com@evil.example/x").is_err());
    }

    #[test]
    fn pulls_the_sentence_out_of_a_worker_error() {
        assert_eq!(
            super::reason(r#"{"error":"that table is far too big"}"#).as_deref(),
            Some("that table is far too big")
        );
        assert_eq!(super::reason("{\"id\":1}"), None);
        assert_eq!(super::reason("not json at all"), None);
    }
}
