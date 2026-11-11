//! Whether an installed service's server could listen on its address, and
//! what to say when it could not (#371). The program tries the address;
//! this crate opens no sockets.

use std::io;
use std::net::SocketAddr;

/// Refuses to install a service whose server could not listen on `bind`,
/// such as an address another program has, before anything is written:
/// installed, it would fail and be started again every few seconds.
/// `try_bind` is the program's answer to whether the server could listen
/// there: it listens on the address and lets it go at once. When the kkt
/// service is `running`, it holds its own address, and installing again
/// replaces it, so the address is not tried. `config_file` is the config
/// file that sets `bind`, if there is one, named in the refusal.
pub fn check_bind(
    running: bool,
    bind: SocketAddr,
    config_file: Option<&str>,
    try_bind: &dyn Fn(SocketAddr) -> io::Result<()>,
) -> Result<(), String> {
    if running {
        return Ok(());
    }
    let Err(e) = try_bind(bind) else {
        return Ok(());
    };
    let set_it = match config_file {
        Some(file) => format!("set another address with `bind` in {file}"),
        None => "set another address with `bind` in a config file".to_string(),
    };
    Err(if e.kind() == io::ErrorKind::AddrInUse {
        format!(
            "{bind} is already in use, so the server could not listen there. Stop the program \
             using it, or {set_it}, then run `kkt service install` again."
        )
    } else {
        format!(
            "the server could not listen on {bind}: {e}. To fix it, {set_it}, then run \
             `kkt service install` again."
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const BIND: &str = "127.0.0.1:7420";

    fn bind() -> SocketAddr {
        BIND.parse().expect("address")
    }

    /// #371: with another program listening on the address, the install
    /// is refused, naming the address and where to change it.
    #[test]
    fn an_address_in_use_is_refused_naming_the_config_file() {
        let in_use = |_: SocketAddr| Err(io::Error::from(io::ErrorKind::AddrInUse));
        assert_eq!(
            check_bind(
                false,
                bind(),
                Some("/home/sam/.config/kkt/config.toml"),
                &in_use
            ),
            Err(format!(
                "{BIND} is already in use, so the server could not listen there. Stop the \
                 program using it, or set another address with `bind` in \
                 /home/sam/.config/kkt/config.toml, then run `kkt service install` again."
            ))
        );
    }

    /// A free address passes.
    #[test]
    fn a_free_address_passes() {
        assert_eq!(check_bind(false, bind(), None, &|_| Ok(())), Ok(()));
    }

    /// The running kkt service holds its own address, and installing again
    /// replaces it, so the address is not tried.
    #[test]
    fn a_running_service_is_not_refused_for_its_own_address() {
        let tried = std::cell::Cell::new(false);
        let try_bind = |_: SocketAddr| {
            tried.set(true);
            Ok(())
        };
        assert_eq!(check_bind(true, bind(), None, &try_bind), Ok(()));
        assert!(!tried.get());
    }

    /// Other reasons the server could not listen are refused too, saying
    /// what the system said.
    #[test]
    fn other_listen_failures_are_refused_with_the_reason() {
        let bind: SocketAddr = "192.0.2.1:7420".parse().expect("address");
        let unavailable = |_: SocketAddr| Err(io::Error::from(io::ErrorKind::AddrNotAvailable));
        let err = check_bind(false, bind, None, &unavailable).expect_err("refused");
        assert!(
            err.starts_with("the server could not listen on 192.0.2.1:7420: "),
            "{err}"
        );
        assert!(
            err.ends_with(
                "To fix it, set another address with `bind` in a config file, then run \
                 `kkt service install` again."
            ),
            "{err}"
        );
    }
}
