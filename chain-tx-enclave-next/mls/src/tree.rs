use std::convert::TryFrom;
use std::fmt;
use std::iter;

use crate::ciphersuite::CipherSuite;
use crate::extensions as ext;
use crate::group::ProcessCommitError;
use crate::key::{HPKEPrivateKey, HPKEPublicKey};
use crate::keypackage::{
    self as kp, KeyPackage, Timespec, MLS10_128_DHKEMP256_AES128GCM_SHA256_P256,
};
use crate::message::DirectPathNode;
use crate::message::*;
use crate::tree_math::{LeafSize, NodeSize, ParentSize};
use crate::utils::{decode_option, encode_option, encode_vec_u32, read_vec_u32};
use chain_util::NonEmpty;
use ra_client::AttestedCertVerifier;
use rustls::internal::msgs::codec::{self, Codec, Reader};
use secrecy::{ExposeSecret, SecretVec};
use subtle::ConstantTimeEq;

#[derive(Clone)]
/// spec: draft-ietf-mls-protocol.md#tree-hashes
pub struct ParentNode {
    pub public_key: HPKEPublicKey,
    // 0..2^32-1
    pub unmerged_leaves: Vec<u32>,
    // 0..255
    pub parent_hash: Vec<u8>,
}

impl fmt::Debug for ParentNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ParentNode")
            .field("public_key", &self.public_key)
            .field("unmerged_leaves", &self.unmerged_leaves)
            .field("parent_hash", &self.parent_hash)
            .finish()
    }
}

impl Codec for ParentNode {
    fn encode(&self, bytes: &mut Vec<u8>) {
        self.public_key.encode(bytes);
        encode_vec_u32(bytes, &self.unmerged_leaves);
        codec::encode_vec_u8(bytes, &self.parent_hash);
    }

    fn read(r: &mut Reader) -> Option<Self> {
        let public_key = HPKEPublicKey::read(r)?;
        let unmerged_leaves = read_vec_u32(r)?;
        let parent_hash = codec::read_vec_u8(r)?;
        Some(Self {
            public_key,
            unmerged_leaves,
            parent_hash,
        })
    }
}

impl ParentNode {
    /// compute the parent hash to be referenced by children
    fn compute_parent_hash(&self, cs: CipherSuite) -> Vec<u8> {
        cs.hash(&self.get_encoding())
    }

    fn set_public_key(&mut self, public_key: HPKEPublicKey) {
        self.public_key = public_key;
        self.unmerged_leaves = vec![];
    }
}

/// spec: draft-ietf-mls-protocol.md#tree-hashes
#[derive(Clone, Debug)]
pub enum Node {
    Leaf(Option<KeyPackage>),
    Parent(Option<ParentNode>),
}

impl Codec for Node {
    fn encode(&self, bytes: &mut Vec<u8>) {
        match self {
            Node::Leaf(kp) => {
                0u8.encode(bytes);
                encode_option(bytes, kp);
            }
            Node::Parent(pn) => {
                1u8.encode(bytes);
                encode_option(bytes, pn);
            }
        }
    }

    fn read(r: &mut Reader) -> Option<Self> {
        let tag = u8::read(r)?;
        match tag {
            0 => {
                let kp: Option<KeyPackage> = decode_option(r)?;
                Some(Node::Leaf(kp))
            }
            1 => {
                let np: Option<ParentNode> = decode_option(r)?;
                Some(Node::Parent(np))
            }
            _ => None,
        }
    }
}

impl Node {
    pub fn is_leaf(&self) -> bool {
        matches!(self, Node::Leaf(_))
    }

    pub fn is_empty_leaf(&self) -> bool {
        matches!(self, Node::Leaf(None))
    }

    pub fn is_empty_node(&self) -> bool {
        matches!(self, Node::Leaf(None) | Node::Parent(None))
    }

    pub fn parent_hash(&self) -> Option<Vec<u8>> {
        match self {
            Node::Leaf(Some(leaf)) => {
                Some(leaf.payload.find_extension::<ext::ParentHashExt>().ok()?.0)
            }
            Node::Parent(Some(parent)) => Some(parent.parent_hash.clone()),
            _ => None,
        }
    }

    /// Only blank node return None
    pub fn public_key(&self) -> Option<&HPKEPublicKey> {
        match self {
            Node::Leaf(Some(leaf)) => Some(&leaf.payload.init_key),
            Node::Parent(Some(parent)) => Some(&parent.public_key),
            _ => None,
        }
    }

    pub fn parent_node(&self) -> Option<&ParentNode> {
        match self {
            Node::Leaf(_) => panic!("invalid node type"),
            Node::Parent(ref pn) => pn.as_ref(),
        }
    }

    fn parent_node_mut(&mut self) -> Option<&mut ParentNode> {
        match self {
            Node::Leaf(_) => panic!("invalid node type"),
            Node::Parent(ref mut pn) => pn.as_mut(),
        }
    }

    /// compute the parent hash to be referenced by children
    ///
    /// blank node return empty vector.
    ///
    /// panic if called on leaf node.
    pub fn compute_parent_hash(&self, cs: CipherSuite) -> Vec<u8> {
        self.parent_node()
            .map(|pn| pn.compute_parent_hash(cs))
            .unwrap_or_default()
    }

    /// merge public key on parent node
    ///
    /// panic for leaf node
    fn merge_public(&mut self, public_key: HPKEPublicKey) {
        match self {
            Node::Leaf(_) => panic!("merge public on leaf node"),
            Node::Parent(None) => {
                *self = Node::Parent(Some(ParentNode {
                    public_key,
                    unmerged_leaves: vec![],
                    parent_hash: vec![],
                }))
            }
            Node::Parent(Some(pn)) => pn.set_public_key(public_key),
        };
    }
}

#[derive(Debug)]
/// spec: draft-ietf-mls-protocol.md#tree-hashes
pub struct ParentNodeHashInput {
    pub node_index: u32,
    pub parent_node: Option<ParentNode>,
    pub left_hash: Vec<u8>,
    pub right_hash: Vec<u8>,
}

impl Codec for ParentNodeHashInput {
    fn encode(&self, bytes: &mut Vec<u8>) {
        self.node_index.encode(bytes);
        encode_option(bytes, &self.parent_node);
        codec::encode_vec_u8(bytes, &self.left_hash);
        codec::encode_vec_u8(bytes, &self.right_hash);
    }

    fn read(r: &mut Reader) -> Option<Self> {
        let node_index = u32::read(r)?;
        let parent_node: Option<ParentNode> = decode_option(r)?;
        let left_hash = codec::read_vec_u8(r)?;
        let right_hash = codec::read_vec_u8(r)?;

        Some(Self {
            node_index,
            parent_node,
            left_hash,
            right_hash,
        })
    }
}

#[derive(Debug)]
/// spec: draft-ietf-mls-protocol.md#tree-hashes
pub struct LeafNodeHashInput {
    pub node_index: u32,
    pub key_package: Option<KeyPackage>,
}

impl Codec for LeafNodeHashInput {
    fn encode(&self, bytes: &mut Vec<u8>) {
        self.node_index.encode(bytes);
        encode_option(bytes, &self.key_package);
    }

    fn read(r: &mut Reader) -> Option<Self> {
        let node_index = u32::read(r)?;
        let key_package: Option<KeyPackage> = decode_option(r)?;

        Some(Self {
            node_index,
            key_package,
        })
    }
}

/// spec: draft-ietf-mls-protocol.md#tree-math
/// TODO: https://github.com/mlswg/mls-protocol/pull/327/files
#[derive(Clone)]
pub struct TreePublicKey {
    /// all tree nodes stored in a vector
    pub nodes: Vec<Node>,
    /// the used ciphersuite (for hashing etc.)
    /// TODO: unify with keypackage one
    pub cs: CipherSuite,
    /// position of the participant in the tree
    pub my_pos: LeafSize,
}

impl TreePublicKey {
    pub fn new(kp: KeyPackage) -> Self {
        TreePublicKey {
            nodes: vec![Node::Leaf(Some(kp))],
            cs: CipherSuite::MLS10_128_DHKEMP256_AES128GCM_SHA256_P256,
            my_pos: LeafSize(0),
        }
    }

    pub fn get_package(&self, leaf_index: LeafSize) -> Option<&KeyPackage> {
        match self.nodes[leaf_index.node_index()] {
            Node::Leaf(ref kp) => kp.as_ref(),
            _ => panic!("invalid node type"),
        }
    }

    pub fn get_package_mut(&mut self, leaf_index: LeafSize) -> Option<&mut KeyPackage> {
        match &mut self.nodes[leaf_index.node_index()] {
            Node::Leaf(kp) => kp.as_mut(),
            _ => panic!("invalid node type"),
        }
    }

    pub fn set_blank(&mut self, index: NodeSize) {
        self.nodes[index.node_index()] = match LeafSize::try_from(index) {
            Ok(_) => Node::Leaf(None),
            _ => Node::Parent(None),
        };
    }

    /// Update keypackage of leaf node
    pub fn set_package(&mut self, leaf_index: LeafSize, kp: KeyPackage) {
        self.nodes[leaf_index.node_index()] = Node::Leaf(Some(kp));
    }

    pub fn get_my_package(&self) -> &KeyPackage {
        self.get_package(self.my_pos).expect("corrupted tree")
    }

    pub fn get_my_package_mut(&mut self) -> &mut KeyPackage {
        self.get_package_mut(self.my_pos).expect("corrupted tree")
    }

    /// Update keypackage of my leaf node
    pub fn set_my_package(&mut self, kp: KeyPackage) {
        self.set_package(self.my_pos, kp)
    }

    pub fn get_parent_node(&self, pos: ParentSize) -> Option<&ParentNode> {
        self.nodes[pos.node_index()].parent_node()
    }

    pub fn from_group_info(
        my_pos: LeafSize,
        cs: CipherSuite,
        nodes: Vec<Node>,
    ) -> Result<Self, kp::Error> {
        // FIXME: generate path secrets
        Ok(Self { nodes, cs, my_pos })
    }

    pub fn integrity_check(
        nodes: &[Node],
        ra_verifier: &impl AttestedCertVerifier,
        time: Timespec,
        cs: CipherSuite,
    ) -> Result<(), TreeIntegrityError> {
        let nodes_len = u32::try_from(nodes.len())
            .map_err(|_| TreeIntegrityError::CorruptedTree("node count overflow"))?;
        let leafs_len = NodeSize(nodes_len)
            .leafs_len()
            .ok_or(TreeIntegrityError::CorruptedTree("node count is not even"))?;

        // leaf should be even position, and parent should be odd position
        for (i, node) in nodes.iter().enumerate() {
            if matches!(node, Node::Leaf(_)) && i % 2 != 0 {
                return Err(TreeIntegrityError::CorruptedTree("leaf index is not even"));
            }
            if matches!(node, Node::Parent(_)) && i % 2 != 1 {
                return Err(TreeIntegrityError::CorruptedTree("parent index is not odd"));
            }
        }

        for (i, node) in nodes.iter().enumerate() {
            match node {
                Node::Leaf(Some(kp)) => {
                    // "For each non-empty leaf node, verify the signature on the KeyPackage."
                    kp.verify(ra_verifier, time)?;
                }
                Node::Parent(Some(_)) => {
                    // "For each non-empty parent node, verify that exactly one of the node's children are non-empty
                    // and have the hash of this node set as their parent_hash value (if the child is another parent)
                    // or has a parent_hash extension in the KeyPackage containing the same value (if the child is a leaf)."
                    let x = NodeSize(i as u32);
                    let px = ParentSize::try_from(x).map_err(|_| {
                        TreeIntegrityError::CorruptedTree("should be parent node index")
                    })?;
                    let left_pos = px.left();
                    let right_pos = px.right(leafs_len);
                    let parent_hash = match (
                        nodes[left_pos.node_index()].parent_hash(),
                        nodes[right_pos.node_index()].parent_hash(),
                    ) {
                        (None, Some(hash)) => hash,
                        (Some(hash), None) => hash,
                        _ => {
                            return Err(TreeIntegrityError::ParentHashEmpty);
                        }
                    };
                    if parent_hash
                        .ct_eq(&node_hash(&nodes, cs, x, leafs_len))
                        .into()
                    {
                        return Err(TreeIntegrityError::ParentHashDontMatch);
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub fn for_group_info(&self) -> Vec<Option<Node>> {
        let mut result = Vec::with_capacity(self.nodes.len());
        for node in self.nodes.iter() {
            if node.is_empty_node() {
                result.push(None)
            } else {
                result.push(Some(node.clone()))
            }
        }
        result
    }

    // failed if node size overflowed u32
    fn get_free_leaf_or_extend(&mut self) -> Result<NodeSize, ()> {
        // nodes length overflow is verified in integrity_check
        match self.nodes.iter().position(|n| n.is_empty_leaf()) {
            Some(i) => Ok(NodeSize(u32::try_from(i).map_err(|_| ())?)),
            None => {
                self.nodes.push(Node::Parent(None));
                self.nodes.push(Node::Leaf(None));
                Ok(NodeSize(
                    u32::try_from(self.nodes.len() - 1).map_err(|_| ())?,
                ))
            }
        }
    }

    pub fn update(
        &mut self,
        adds: &[Add],
        updates: &[(LeafSize, Update, ProposalId)],
        removes: &[Remove],
    ) -> Vec<(NodeSize, KeyPackage)> {
        // spec: draft-ietf-mls-protocol.md#update
        for (sender, update, _) in updates.iter() {
            self.set_package(*sender, update.key_package.clone());
            for p in NodeSize::from(*sender).direct_path(self.leaf_len()) {
                self.set_blank(p.into());
            }
        }
        // spec: draft-ietf-mls-protocol.md#remove
        for remove in removes.iter() {
            let removed = NodeSize::from(LeafSize(remove.removed));
            self.set_blank(removed);
            for p in removed.direct_path(self.leaf_len()) {
                self.set_blank(p.into());
            }
        }
        let mut positions = Vec::with_capacity(adds.len());
        for add in adds.iter() {
            // position to add
            // "If necessary, extend the tree to the right until it has at least index + 1 leaves"
            let position = self
                .get_free_leaf_or_extend()
                .expect("node size overflowed u32");
            let leafs = self.leaf_len();
            let dirpath = position.direct_path(leafs);
            for d in dirpath.iter() {
                let node = &mut self.nodes[d.node_index()];
                if let Node::Parent(Some(ref mut np)) = node {
                    // "For each non-blank intermediate node along the
                    // path from the leaf at position index to the root,
                    // add index to the unmerged_leaves list for the node."
                    if !np.unmerged_leaves.contains(&d.0) {
                        np.unmerged_leaves.push(d.0);
                    }
                }
            }
            // "Set the leaf node in the tree at position index to a new node containing the public key
            // from the KeyPackage in the Add, as well as the credential under which the KeyPackage was signed"
            let leaf_node = Node::Leaf(Some(add.key_package.clone()));
            self.nodes[position.node_index()] = leaf_node;
            positions.push((position, add.key_package.clone()));
        }
        positions
    }

    fn node_hash(&self, index: NodeSize) -> Vec<u8> {
        node_hash(&self.nodes, self.cs, index, self.leaf_len())
    }

    pub fn leaf_len(&self) -> LeafSize {
        // node count is verified in integrity_check
        NodeSize(self.nodes.len() as u32)
            .leafs_len()
            .expect("invalid node count")
    }

    pub fn compute_tree_hash(&self) -> Vec<u8> {
        let root_index = NodeSize::root(self.leaf_len());
        self.node_hash(root_index)
    }

    pub fn init(creator_kp: KeyPackage) -> Self {
        let cs = if creator_kp.payload.cipher_suite == MLS10_128_DHKEMP256_AES128GCM_SHA256_P256 {
            CipherSuite::MLS10_128_DHKEMP256_AES128GCM_SHA256_P256
        } else {
            panic!("unify the cipher suite representation")
        };
        Self {
            nodes: vec![Node::Leaf(Some(creator_kp))],
            // FIXME unify cipher_suite
            cs,
            my_pos: LeafSize(0),
        }
    }

    /// spec: draft-ietf-mls-protocol.md#ratchet-tree-evolution
    ///
    /// update path secrets
    ///
    /// returns: direct path nodes, parent_hash, tree_secret
    pub fn evolve(
        &mut self,
        ctx: &[u8],
        leaf_secret: &SecretVec<u8>,
    ) -> (Vec<DirectPathNode>, Vec<u8>, TreeSecret) {
        let tree_secret =
            TreeSecret::new(self.cs, self.my_pos.into(), self.leaf_len(), &leaf_secret)
                .expect("invalid length");
        let leaf_len = self.leaf_len();
        let my_node = NodeSize::from(self.my_pos);
        let path = my_node.direct_path(leaf_len);
        let mut path_nodes = vec![];

        // set path secrets up the tree and encrypt to the siblings.
        // the root node is the last node of path,
        // and n won't be the root node because of skip(1) in iterator.
        let full_path = iter::once(my_node).chain(path.iter().copied().map(NodeSize::from));
        let path_with_secrets = path.iter().copied().zip(tree_secret.path_secrets.iter());
        for (node, (parent, secret)) in full_path.zip(path_with_secrets) {
            // encrypt the secret to resolution maintained
            let sibling = node.sibling(leaf_len).expect("not root node");
            let encrypted_path_secret = resolve(&self.nodes, sibling)
                .iter()
                .map(|node| {
                    // encrypt to sibling's public key
                    let init_key = self.nodes[node.node_index()]
                        .public_key()
                        .expect("not blank node");
                    self.cs
                        .encrypt(secret.expose_secret().clone(), &init_key, ctx)
                        .expect("encrypt failed")
                })
                .collect::<Vec<_>>();

            // derive keypair for parent
            let private_key = self.cs.derive_private_key(&secret);
            let public_key = private_key.public_key();

            path_nodes.push(DirectPathNode {
                public_key: public_key.clone(),
                encrypted_path_secret,
            });

            // update the parent node
            self.nodes[parent.node_index()] = Node::Parent(Some(ParentNode {
                public_key,
                unmerged_leaves: vec![],
                parent_hash: vec![], // will set later
            }));
        }

        // update parent hash in path
        let leaf_parent_hash = self.set_parent_hash_path(self.my_pos);

        (path_nodes, leaf_parent_hash, tree_secret)
    }

    /// merge parent node public keys from `DirectPath`
    pub fn merge(&mut self, sender: LeafSize, path_nodes: &[DirectPathNode]) -> Vec<u8> {
        let leaf_len = self.leaf_len();
        let from = NodeSize::from(sender);
        // set public key
        for (node_index, path_node) in from
            .direct_path(leaf_len)
            .into_iter()
            .zip(path_nodes.iter())
        {
            self.nodes[node_index.node_index()].merge_public(path_node.public_key.clone());
        }

        // update parent hash
        self.set_parent_hash_path(sender)
    }

    fn set_parent_hash_path(&mut self, sender: LeafSize) -> Vec<u8> {
        let from = NodeSize::from(sender);
        let path = from.direct_path(self.leaf_len());
        for (node, parent) in path.iter().copied().zip(path.iter().skip(1).copied()).rev() {
            // no leaf node
            let hash = self.nodes[parent.node_index()].compute_parent_hash(self.cs);
            if let Some(n) = self.nodes[node.node_index()].parent_node_mut() {
                n.parent_hash = hash;
            }
        }
        self.nodes[path[0].node_index()].compute_parent_hash(self.cs)
    }

    /// Verify the secret match the public key in the parent node
    pub(crate) fn verify_node_private_key(
        &self,
        secret: &SecretVec<u8>,
        pos: ParentSize,
    ) -> Result<(), ()> {
        if let Some(node) = self.get_parent_node(pos) {
            let private_key = self.cs.derive_private_key(secret);
            if bool::from(
                private_key
                    .public_key()
                    .marshal()
                    .ct_eq(&node.public_key.marshal()),
            ) {
                return Ok(());
            }
        }
        Err(())
    }
}

#[derive(thiserror::Error, Debug)]
pub enum TreeIntegrityError {
    #[error("keypackage verify failed: {0}")]
    KeyPackageVerifyFail(#[from] kp::Error),
    #[error("corrupted tree structure: {0}")]
    CorruptedTree(&'static str),
    #[error("children don't have parent hash")]
    ParentHashEmpty,
    #[error("parent hash value don't match")]
    ParentHashDontMatch,
}

/// Store the path secrets for a leaf node.
///
/// For example, in this tree:
///
/// ```plain
///                                              X
///                      X
///          X                       X                       X
///    X           X           X           X           X
/// X     X     X     X     X     X     X     X     X     X     X
/// 0  1  2  3  4  5  6  7  8  9 10 11 12 13 14 15 16 17 18 19 20
/// ```
///
/// The direct path of leaf 0 is [1, 3, 7, 15], the secret of the nodes in the direct path is
/// stored in `path_secrets`, and the secrets for the leaf node itself is stored in `kp_secret` in
/// `GroupAux`.
///
/// The `update_secret` is derived from the secret of the root node.
pub struct TreeSecret {
    /// Not including leaf private key
    pub path_secrets: Vec<SecretVec<u8>>,
    /// The commit secret
    pub update_secret: SecretVec<u8>,
}

impl TreeSecret {
    pub fn new(
        cs: CipherSuite,
        start: NodeSize,
        leaf_len: LeafSize,
        secret: &SecretVec<u8>,
    ) -> Result<Self, hkdf::InvalidLength> {
        let mut path_secrets = vec![];
        for _ in start.direct_path(leaf_len).iter() {
            // path secret for parent
            path_secrets.push(cs.expand_label(
                path_secrets.last().unwrap_or(secret),
                vec![],
                "path",
                &[],
                cs.secret_size(),
            )?);
        }

        let update_secret = cs.expand_label(
            &path_secrets.last().unwrap_or(secret),
            vec![],
            "path",
            &[],
            cs.secret_size(),
        )?;

        Ok(Self {
            path_secrets,
            update_secret,
        })
    }

    pub fn empty(cs: CipherSuite) -> Self {
        Self {
            path_secrets: vec![],
            update_secret: SecretVec::new(vec![0; cs.hash_len()]),
        }
    }

    /// find the intersection between sender and self.my_pos,
    /// and decrypt the path secret.
    ///
    /// returns None if tree only have one leaf
    fn decrypt_path_secrets(
        &self,
        sender: LeafSize,
        tree: &TreePublicKey,
        ctx: &[u8],
        path_nodes: &[DirectPathNode],
        leaf_private_key: &HPKEPrivateKey,
    ) -> Result<Option<(ParentSize, SecretVec<u8>)>, ProcessCommitError> {
        let leaf_len = tree.leaf_len();
        let ancestor = ParentSize::common_ancestor(sender, tree.my_pos);
        match ancestor {
            None => {
                // `sender` and `my_pos` are the same leaf node
                let node_index = NodeSize::from(sender);
                match node_index.parent(leaf_len) {
                    Some(node) => Ok(Some((
                        node,
                        tree.cs.expand_label(
                            &leaf_private_key.marshal(),
                            vec![],
                            "path",
                            &[],
                            tree.cs.secret_size(),
                        )?,
                    ))),
                    None => Ok(None),
                }
            }
            Some(ancestor) => {
                // find the path node correspounding to the ancestor
                let path_node = NodeSize::from(sender)
                    .direct_path(leaf_len)
                    .into_iter()
                    .zip(path_nodes.iter())
                    .find(|(n, _)| *n == ancestor)
                    .expect("invalid path nodes")
                    .1;
                // find the index of ancestor in the direct_path/path_secrets
                let pos = NodeSize::from(tree.my_pos)
                    .direct_path(leaf_len)
                    .iter()
                    .position(|&p| p == ancestor)
                    .expect("ancestor in direct path");
                // move one level down from ancestor to get to the node the secret encrypted to,
                // `None` means it's leaf node under the ancestor.
                let secret = match pos.checked_sub(1) {
                    None => tree.cs.decrypt(
                        &leaf_private_key,
                        ctx,
                        &path_node.encrypted_path_secret[0],
                    )?,
                    Some(pos) => tree.cs.decrypt(
                        &HPKEPrivateKey::unmarshal(self.path_secrets[pos].expose_secret())
                            .expect("invalid secret"),
                        ctx,
                        &path_node.encrypted_path_secret[0],
                    )?,
                };
                Ok(Some((ancestor, SecretVec::new(secret))))
            }
        }
    }

    /// draft-ietf-mls-protocol.md#synchronizing-views-of-the-tree
    ///
    /// find the overlap parent node, decrypt the secret and implant it
    ///
    /// it don't mutate self directly, but return a `TreeSecretDiff` which can be applied to
    /// `TreeSecret` later.
    pub(crate) fn apply_path_secrets(
        &self,
        sender: LeafSize,
        tree: &TreePublicKey,
        ctx: &[u8],
        path_nodes: &[DirectPathNode],
        leaf_private_key: &HPKEPrivateKey,
    ) -> Result<TreeSecretDiff, ProcessCommitError> {
        let leaf_len = tree.leaf_len();
        // Find overlap and decrypt the path secret
        if let Some((overlap, overlap_secret)) =
            self.decrypt_path_secrets(sender, tree, ctx, path_nodes, leaf_private_key)?
        {
            let direct_path = NodeSize::from(tree.my_pos).direct_path(leaf_len);
            let overlap_pos = direct_path
                .iter()
                .position(|&p| p == overlap)
                .expect("overlap is supposed to be ancestor");
            let overlap_path = &direct_path[overlap_pos + 1..];
            debug_assert_eq!(
                overlap_path.iter().copied().collect::<Vec<_>>(),
                NodeSize::from(overlap).direct_path(leaf_len)
            );

            // verify the new path secrets match public keys
            tree.verify_node_private_key(&overlap_secret, overlap)
                .map_err(|_| ProcessCommitError::PathSecretPublicKeyDontMatch)?;
            // the path secrets above(including) the overlap node
            let mut path_secrets = NonEmpty::from(overlap_secret);
            for _ in overlap_path.iter() {
                path_secrets.push(tree.cs.expand_label(
                    path_secrets.last(),
                    vec![],
                    "path",
                    &[],
                    tree.cs.secret_size(),
                )?);
            }
            assert_eq!(overlap_pos + path_secrets.len().get(), direct_path.len());

            // verify the new path secrets match public keys
            for (secret, &parent) in path_secrets.iter().skip(1).zip(overlap_path) {
                tree.verify_node_private_key(secret, parent)
                    .map_err(|_| ProcessCommitError::PathSecretPublicKeyDontMatch)?;
            }

            // Define `commit_secret` as the value `path_secret[n+1]` derived from the
            // `path_secret[n]` value assigned to the root node.
            let update_secret = tree.cs.expand_label(
                path_secrets.last(),
                vec![],
                "path",
                &[],
                tree.cs.secret_size(),
            )?;

            Ok(TreeSecretDiff {
                overlap_pos: Some(overlap_pos),
                path_secrets: path_secrets.into(),
                update_secret,
            })
        } else {
            let update_secret = tree.cs.expand_label(
                &leaf_private_key.marshal(),
                vec![],
                "path",
                &[],
                tree.cs.secret_size(),
            )?;
            Ok(TreeSecretDiff {
                overlap_pos: None,
                path_secrets: vec![],
                update_secret,
            })
        }
    }

    pub(crate) fn apply_tree_diff(&mut self, tree_diff: TreeSecretDiff) {
        if let Some(overlap_pos) = tree_diff.overlap_pos {
            self.path_secrets.truncate(overlap_pos);
            self.path_secrets.extend(tree_diff.path_secrets);
        }
        self.update_secret = tree_diff.update_secret;
    }
}

/// Represent the diff needs to be applied later to `TreeSecret`
pub struct TreeSecretDiff {
    /// the nodes from overlap_pos to root need to be updated
    ///
    /// None means no common ancestor
    pub overlap_pos: Option<usize>,
    /// the path secrets from(including) overlap_pos to root
    pub path_secrets: Vec<SecretVec<u8>>,
    pub update_secret: SecretVec<u8>,
}

/// draft-ietf-mls-protocol.md#tree-hashes
fn node_hash(nodes: &[Node], cs: CipherSuite, index: NodeSize, leaf_size: LeafSize) -> Vec<u8> {
    let node = &nodes[index.node_index()];
    let payload = match node {
        Node::Leaf(kp) => LeafNodeHashInput {
            node_index: index.0,
            key_package: kp.clone(),
        }
        .get_encoding(),
        Node::Parent(pn) => {
            let pindex = ParentSize::try_from(index).expect("must be parent node index");
            let left_index = pindex.left();
            let left_hash = node_hash(nodes, cs, left_index, leaf_size);
            let right_index = pindex.right(leaf_size);
            let right_hash = node_hash(nodes, cs, right_index, leaf_size);
            ParentNodeHashInput {
                node_index: index.0,
                parent_node: pn.clone(),
                left_hash,
                right_hash,
            }
            .get_encoding()
        }
    };
    cs.hash(&payload)
}

/// draft-ietf-mls-protocol.md#ratchet-tree-nodes
/// no blank nodes return
fn resolve(nodes: &[Node], index: NodeSize) -> Vec<NodeSize> {
    match &nodes[index.node_index()] {
        // Resolution of blank leaf is the empty list
        Node::Leaf(None) => vec![],
        // Resolution of non-blank leaf is node itself
        Node::Leaf(Some(_)) => vec![index],
        // Resolution of blank intermediate node is concatenation of the resolutions
        // of the children
        Node::Parent(None) => {
            let pindex = ParentSize::try_from(index).expect("must be parent node index");
            [
                resolve(nodes, pindex.left()),
                resolve(
                    nodes,
                    pindex.right(
                        NodeSize(nodes.len() as u32)
                            .leafs_len()
                            .expect("invalid node size"),
                    ),
                ),
            ]
            .concat()
        }
        // Resolution of non-blank leaf is node + unmerged leaves
        Node::Parent(Some(p)) => iter::once(index)
            .chain(
                p.unmerged_leaves
                    .iter()
                    .copied()
                    .map(|n| NodeSize::from(LeafSize(n))),
            )
            .collect::<Vec<_>>(),
    }
}
