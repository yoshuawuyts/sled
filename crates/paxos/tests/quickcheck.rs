#[macro_use]
extern crate quickcheck;
extern crate rand;
extern crate paxos;

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};
use std::ops::Add;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use self::paxos::*;
use self::quickcheck::{Arbitrary, Gen};

#[derive(PartialOrd, Ord, Eq, PartialEq, Debug, Clone)]
enum ClientRequest {
    Get,
    Set(Vec<u8>),
    Cas(Option<Vec<u8>>, Option<Vec<u8>>),
    Del,
}

impl Arbitrary for ClientRequest {
    fn arbitrary<G: Gen>(g: &mut G) -> Self {
        let choice = g.gen_range(0, 4);

        match choice {
            0 => ClientRequest::Get,
            1 => ClientRequest::Set(vec![g.gen_range(0, 5)]),
            2 => {
                ClientRequest::Cas(
                    if g.gen() {
                        Some(vec![g.gen_range(0, 5)])
                    } else {
                        None
                    },
                    if g.gen() {
                        Some(vec![g.gen_range(0, 5)])
                    } else {
                        None
                    },
                )
            }
            3 => ClientRequest::Del,
            _ => panic!("somehow generated 3+..."),
        }
    }
}

#[derive(Eq, PartialEq, Debug, Clone)]
struct ScheduledMessage {
    at: SystemTime,
    from: String,
    to: String,
    msg: Rpc,
}

// we implement Ord and PartialOrd to make the BinaryHeap
// act like a min-heap on time, rather than the default
// max-heap, so time progresses forwards.
impl Ord for ScheduledMessage {
    fn cmp(&self, other: &ScheduledMessage) -> Ordering {
        other.at.cmp(&self.at)
    }
}

impl PartialOrd for ScheduledMessage {
    fn partial_cmp(&self, other: &ScheduledMessage) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone)]
enum Node {
    Acceptor(Acceptor),
    Proposer(Proposer),
    Client(Client),
}

impl Reactor for Node {
    type Peer = String;
    type Message = Rpc;

    fn receive(
        &mut self,
        at: SystemTime,
        from: Self::Peer,
        msg: Self::Message,
    ) -> Vec<(Self::Peer, Self::Message)> {
        match *self {
            Node::Proposer(ref mut inner) => inner.receive(at, from, msg),
            Node::Acceptor(ref mut inner) => inner.receive(at, from, msg),
            Node::Client(ref mut inner) => inner.receive(at, from, msg),
        }
    }
}

#[derive(Debug, Clone)]
struct Cluster {
    peers: HashMap<String, Node>,
    omniscient_time: u64,
    in_flight: BinaryHeap<ScheduledMessage>,
    client_responses: Vec<ScheduledMessage>,
}

impl Cluster {
    fn step(&mut self) -> Option<()> {
        let pop = self.in_flight.pop();
        if let Some(sm) = pop {
            if sm.to.starts_with("client:") {
                // We'll check linearizability later
                // for client responses.
                self.client_responses.push(sm);
                return Some(());
            }
            let node = self.peers.get_mut(&sm.to).unwrap();
            let at = sm.at.clone();
            for (to, msg) in node.receive(sm.at, sm.from, sm.msg) {
                // TODO partitions
                // TODO clock messin'
                let new_sm = ScheduledMessage {
                    at: at.add(Duration::new(0, 1)),
                    from: sm.to.clone(),
                    to: to,
                    msg: msg,
                };
                self.in_flight.push(new_sm);
            }
            Some(())
        } else {
            None
        }
    }
}

unsafe impl Send for Cluster {}

impl Arbitrary for Cluster {
    fn arbitrary<G: Gen>(g: &mut G) -> Self {
        let n_clients = g.gen_range(1, 4);
        let client_addrs: Vec<String> =
            (0..n_clients).map(|i| format!("client:{}", i)).collect();

        let n_proposers = g.gen_range(1, 4);
        let proposer_addrs: Vec<String> = (0..n_proposers)
            .map(|i| format!("proposer:{}", i))
            .collect();

        let n_acceptors = g.gen_range(1, 4);
        let acceptor_addrs: Vec<String> = (0..n_acceptors)
            .map(|i| format!("acceptor:{}", i))
            .collect();

        let clients: Vec<(String, Node)> = client_addrs
            .iter()
            .map(|addr| {
                (
                    addr.clone(),
                    Node::Client(Client::new(proposer_addrs.clone())),
                )
            })
            .collect();

        let proposers: Vec<(String, Node)> = proposer_addrs
            .iter()
            .map(|addr| {
                (
                    addr.clone(),
                    Node::Proposer(Proposer::new(acceptor_addrs.clone())),
                )
            })
            .collect();

        let acceptors: Vec<(String, Node)> = acceptor_addrs
            .iter()
            .map(|addr| (addr.clone(), Node::Acceptor(Acceptor::default())))
            .collect();

        let mut requests = vec![];

        for client_addr in client_addrs {
            let n_requests = g.gen_range(1, 10);

            for r in 0..n_requests {
                let msg = match ClientRequest::arbitrary(g) {
                    ClientRequest::Get => Rpc::Get(r),
                    ClientRequest::Set(v) => Rpc::Set(r, v),
                    ClientRequest::Cas(ov, nv) => Rpc::Cas(r, ov, nv),
                    ClientRequest::Del => Rpc::Del(r),
                };

                let at = g.gen_range(0, 100);

                requests.push(ScheduledMessage {
                    at: UNIX_EPOCH.add(Duration::new(0, at)),
                    from: client_addr.clone(),
                    to: g.choose(&proposer_addrs).unwrap().clone(),
                    msg: msg,
                });
            }
        }

        Cluster {
            peers: clients
                .into_iter()
                .chain(proposers.into_iter())
                .chain(acceptors.into_iter())
                .collect(),
            omniscient_time: 0,
            in_flight: requests.clone().into_iter().collect(),
            client_responses: vec![],
        }
    }
}

fn check_linearizability(
    requests: Vec<ScheduledMessage>,
    responses: Vec<ScheduledMessage>,
) -> bool {

    true
}

quickcheck! {
    fn cluster_linearizability(cluster: Cluster) -> bool {
        let mut cluster = cluster;
        let client_requests: Vec<_> = cluster.in_flight
            .clone()
            .into_iter()
            .collect();

        while let Some(_) = cluster.step() {} 

        check_linearizability(client_requests, cluster.client_responses)
    }
}
