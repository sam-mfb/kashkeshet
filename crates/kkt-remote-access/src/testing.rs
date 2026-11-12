//! Remote sharing in memory for consumers' tests, without network or processes.

use crate::{Node, RemoteAccess};
use std::cell::RefCell;
use std::net::SocketAddr;

/// A machine with the given network name, initially sharing nothing.
pub struct Memory {
    node: Node,
    shared: RefCell<Option<SocketAddr>>,
}

impl Memory {
    pub fn new(node: Node) -> Self {
        Self {
            node,
            shared: RefCell::new(None),
        }
    }
}

impl RemoteAccess for Memory {
    fn prepare(&self) -> Result<Node, String> {
        Ok(self.node.clone())
    }
    fn serve(&self, bind: SocketAddr) -> Result<(), String> {
        let mut shared = self.shared.borrow_mut();
        if shared.is_some_and(|existing| existing != bind) {
            return Err("another server is already shared".into());
        }
        *shared = Some(bind);
        Ok(())
    }
    fn served(&self, bind: SocketAddr) -> Result<Option<Node>, String> {
        Ok((*self.shared.borrow() == Some(bind)).then(|| self.node.clone()))
    }
    fn stop(&self, bind: SocketAddr) -> Result<(), String> {
        if *self.shared.borrow() == Some(bind) {
            *self.shared.borrow_mut() = None;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preparing_and_stopping_another_server_preserve_the_sharing() {
        let node = Node {
            dns_name: "this-machine.private-network".into(),
        };
        let sharing = Memory::new(node.clone());
        let server = "127.0.0.1:7420".parse().expect("address");
        let another = "127.0.0.1:7421".parse().expect("another address");
        assert_eq!(sharing.prepare(), Ok(node.clone()));
        assert_eq!(sharing.served(server), Ok(None));
        sharing.serve(server).expect("share");
        assert_eq!(sharing.prepare(), Ok(node.clone()));
        assert!(
            sharing.serve(another).is_err(),
            "do not replace another server"
        );
        sharing.stop(another).expect("leave another server alone");
        assert_eq!(sharing.served(server), Ok(Some(node)));
        assert_eq!(sharing.served(another), Ok(None));
        sharing.stop(server).expect("stop this server");
        assert_eq!(sharing.served(server), Ok(None));
    }
}
