//! Choosing the port. Porchly never needs administrator rights: the default port is above
//! 1024, and when it is busy Porchly moves on to the next one instead of failing.

use std::io::ErrorKind;
use std::net::{IpAddr, TcpListener};

use porchly_core::{Error, Result};

/// How many ports to try, starting at the requested one, when the exact port isn't required.
pub const PORT_TRIES: u16 = 20;

/// Binds the API's socket. With `exact`, only `port` is tried; otherwise `port`, `port + 1`, …
/// up to [`PORT_TRIES`] ports, skipping ones that are busy or reserved. Errors are written for
/// people: they say what went wrong and what to do.
pub fn bind(host: IpAddr, port: u16, exact: bool) -> Result<TcpListener> {
    if port == 0 {
        return Err(Error::invalid("Port 0 isn't a real port. Leave it empty to use Porchly's usual port, 11911."));
    }
    let tries = if exact { 1 } else { PORT_TRIES };
    let mut last_busy = port;
    for offset in 0..tries {
        let Some(p) = port.checked_add(offset) else { break };
        match TcpListener::bind((host, p)) {
            Ok(listener) => return Ok(listener),
            Err(e) if e.kind() == ErrorKind::PermissionDenied && p < 1024 => {
                return Err(Error::invalid(format!(
                    "Port {p} needs administrator rights. Porchly doesn't need them: use a port above 1024, or leave it empty to use 11911."
                )));
            }
            // Busy, or reserved by the system (Windows reports reserved ranges as "permission denied").
            Err(e) if matches!(e.kind(), ErrorKind::AddrInUse | ErrorKind::PermissionDenied) => {
                last_busy = p;
                if exact {
                    return Err(Error::Conflict(format!(
                        "Port {p} is already being used by another app. Choose another port, or leave it empty and Porchly will find a free one."
                    )));
                }
            }
            Err(e) if e.kind() == ErrorKind::AddrNotAvailable => {
                return Err(Error::invalid(format!(
                    "This computer doesn't have the address {host}. Leave it empty to only accept connections from this computer."
                )));
            }
            Err(e) => return Err(Error::invalid(format!("Couldn't use port {p}: {e}"))),
        }
    }
    Err(Error::Conflict(format!("Ports {port} to {last_busy} are all being used by other apps. Choose another port.")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    const LOCAL: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

    /// A port that is free right now, found by asking the system for one.
    fn free_port() -> u16 {
        TcpListener::bind((LOCAL, 0)).unwrap().local_addr().unwrap().port()
    }

    #[test]
    fn falls_back_when_busy() {
        let busy = TcpListener::bind((LOCAL, 0)).unwrap();
        let port = busy.local_addr().unwrap().port();
        if port.checked_add(PORT_TRIES).is_none() {
            return;
        }
        let got = bind(LOCAL, port, false).unwrap();
        assert_ne!(got.local_addr().unwrap().port(), port);
    }

    #[test]
    fn exact_port_reports_busy() {
        let busy = TcpListener::bind((LOCAL, 0)).unwrap();
        let port = busy.local_addr().unwrap().port();
        let err = bind(LOCAL, port, true).unwrap_err();
        assert!(matches!(err, Error::Conflict(_)));
        assert!(err.to_string().contains("another app"));
    }

    #[test]
    fn free_port_is_used_as_asked() {
        let port = free_port();
        assert_eq!(bind(LOCAL, port, true).unwrap().local_addr().unwrap().port(), port);
    }

    #[test]
    fn low_ports_explain_admin_rights() {
        // Only meaningful when this process can't bind low ports (not root).
        if TcpListener::bind((LOCAL, 80)).is_ok() {
            return;
        }
        let err = bind(LOCAL, 80, false).unwrap_err();
        assert!(err.to_string().contains("administrator"), "{err}");
    }

    #[test]
    fn port_zero_is_refused() {
        assert!(bind(LOCAL, 0, false).is_err());
    }
}
