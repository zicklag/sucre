//! Definitions of the runtime memory types.

/// The size of an endpoint in memory.
pub const ENDPOINT_SIZE: usize = std::mem::size_of::<Endpoint>();

bitfield::bitfield! {
    /// An endpoint in the graph.
    ///
    /// Endpoints may be ports on a node, ends of a wire, or simply a router endpoint used to
    /// temporarily connect nodes.
    #[repr(transparent)]
    #[derive(Clone, Copy)]
    pub struct Endpoint(u64);
    impl Debug;

    /// The kind of endpoint this is.
    pub from into EndpointKind, kind, set_kind: 2, 0;
    /// The data associated to the endpoint.
    pub data, set_data: 63, 3;
}

/// the kind of
#[derive(Debug)]
pub enum EndpointKind {
    /// There is no endpoint here. I.e this is like `None` or a null pointer.
    None,
    /// One of the ends of a wire.
    Wire,
    /// A port on a constructor or duplicator node.
    Node,
    /// An eraser node.
    Eraser,
}

impl From<u64> for EndpointKind {
    fn from(value: u64) -> Self {
        match value {
            0 => Self::None,
            1 => Self::Wire,
            2 => Self::Node,
            3 => Self::Eraser,
            v => panic!("Invalid int conversion to `EndpointKind`: {v}"),
        }
    }
}

impl From<EndpointKind> for u64 {
    fn from(value: EndpointKind) -> Self {
        match value {
            EndpointKind::None => 0,
            EndpointKind::Wire => 1,
            EndpointKind::Node => 2,
            EndpointKind::Eraser => 3,
        }
    }
}

/// The different kinds of memory cells.
#[derive(Debug)]
pub enum NodeKind {
    /// A constructor node.
    Constructor,
    /// A duplicator node.
    Duplicator,
    /// An eraser node.
    Eraser,
    /// An external data node.
    ExternData,
    /// And external function node.
    ExternFn,
}

impl From<u64> for NodeKind {
    fn from(value: u64) -> Self {
        match value {
            0 => Self::Constructor,
            1 => Self::Duplicator,
            2 => Self::Eraser,
            3 => Self::ExternData,
            4 => Self::ExternFn,
            _ => panic!("Invalid value for `MemoryCellKind`: {value}"),
        }
    }
}

impl From<NodeKind> for u64 {
    fn from(val: NodeKind) -> Self {
        match val {
            NodeKind::Constructor => 0,
            NodeKind::Duplicator => 1,
            NodeKind::Eraser => 2,
            NodeKind::ExternData => 3,
            NodeKind::ExternFn => 4,
        }
    }
}
