use crate::ciphersuite::*;
use crate::extensions as ext;
use crate::key::IdentityPublicKey;
use crate::keypackage::Timespec;
use crate::keypackage::PROTOCOL_VERSION_MLS10;
use crate::keypackage::{self as kp, KeyPackage, OwnedKeyPackage};
use crate::message::*;
use crate::secrets::*;
use crate::tree::*;
use crate::tree_math::{LeafSize, NodeSize};
use crate::utils::{
    encode_vec_option_u32, encode_vec_u8_u16, encode_vec_u8_u8, read_vec_option_u32,
    read_vec_u8_u16, read_vec_u8_u8,
};
use ra_client::AttestedCertVerifier;
use rustls::internal::msgs::codec::{self, Codec, Reader};
use secrecy::{ExposeSecret, SecretVec};
use sha2::Sha256;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use subtle::ConstantTimeEq;

/// auxiliary structure to hold group context + tree
pub struct GroupAux {
    pub context: GroupContext,
    pub tree: Tree,
    pub secrets: EpochSecrets<Sha256>,
    pub owned_kp: OwnedKeyPackage,
}

impl GroupAux {
    fn new(context: GroupContext, tree: Tree, owned_kp: OwnedKeyPackage) -> Self {
        let secrets: EpochSecrets<Sha256> = match &tree.cs {
            CipherSuite::MLS10_128_DHKEMP256_AES128GCM_SHA256_P256 => {
                EpochSecrets::new(tree.cs.hash(&context.get_encoding()), tree.leaf_len())
            }
        };
        GroupAux {
            context,
            tree,
            secrets,
            owned_kp,
        }
    }

    fn get_sender(&self) -> Sender {
        Sender {
            sender_type: SenderType::Member,
            sender: self.tree.my_pos.0 as u32,
        }
    }

    fn get_signed_add(&self, kp: &KeyPackage) -> MLSPlaintext {
        let sender = self.get_sender();
        let add_content = MLSPlaintextCommon {
            group_id: self.context.group_id.clone(),
            epoch: self.context.epoch,
            sender,
            authenticated_data: vec![],
            content: ContentType::Proposal(Proposal::Add(Add {
                key_package: kp.clone(),
            })),
        };
        let to_be_signed = MLSPlaintextTBS {
            context: self.context.clone(),
            content: add_content.clone(),
        }
        .get_encoding();
        let signature = self.owned_kp.credential_private_key.sign(&to_be_signed);
        MLSPlaintext {
            content: add_content,
            signature,
        }
    }

    fn get_init_confirmed_transcript_hash(&self, commit: &Commit) -> Vec<u8> {
        let interim_transcript_hash = b"".to_vec(); // TODO
        let content_to_commit = MLSPlaintextCommitContent::new(
            self.context.group_id.clone(),
            self.context.epoch,
            self.get_sender(),
            commit.clone(),
        )
        .get_encoding();
        let to_hash = [interim_transcript_hash, content_to_commit].concat();
        self.tree.cs.hash(&to_hash)
    }

    fn get_interim_transcript_hash(
        &self,
        commit_confirmation: Vec<u8>,
        commit_msg_sig: Vec<u8>,
        confirmed_transcript: Vec<u8>,
    ) -> Vec<u8> {
        let commit_auth = MLSPlaintextCommitAuthData {
            confirmation: commit_confirmation,
            signature: commit_msg_sig,
        }
        .get_encoding();
        self.tree
            .cs
            .hash(&[confirmed_transcript, commit_auth].concat())
    }

    fn get_signed_commit(&self, plain: &MLSPlaintextCommon) -> MLSPlaintext {
        let to_be_signed = MLSPlaintextTBS {
            context: self.context.clone(), // TODO: current or next context?
            content: plain.clone(),
        }
        .get_encoding();
        let signature = self.owned_kp.credential_private_key.sign(&to_be_signed);
        MLSPlaintext {
            content: plain.clone(),
            signature,
        }
    }

    fn get_welcome_msg(
        &self,
        updated_tree: &Tree,
        updated_group_context: &GroupContext,
        updated_secrets: &EpochSecrets<Sha256>,
        confirmation: Vec<u8>,
        interim_transcript_hash: Vec<u8>,
        positions: Vec<(NodeSize, KeyPackage)>,
    ) -> Welcome {
        let group_info_p = GroupInfoPayload {
            group_id: updated_group_context.group_id.clone(),
            epoch: updated_group_context.epoch,
            tree: updated_tree.for_group_info(),
            confirmed_transcript_hash: updated_group_context.confirmed_transcript_hash.clone(),
            interim_transcript_hash,
            extensions: updated_group_context.extensions.clone(), // FIXME: gen new keypackage + extension with parent hash?
            confirmation,
            signer_index: self.get_sender().sender,
        };
        let signature = self
            .owned_kp
            .credential_private_key
            .sign(&group_info_p.get_encoding());
        let group_info = GroupInfo {
            payload: group_info_p,
            signature,
        };
        let (welcome_key, welcome_nonce) = updated_secrets.get_welcome_secret_key_nonce(
            self.tree.cs.aead_key_len(),
            self.tree.cs.aead_nonce_len(),
        );
        let encrypted_group_info =
            self.tree
                .cs
                .encrypt_group_info(&group_info, welcome_key, welcome_nonce);
        let mut secrets = Vec::with_capacity(positions.len());
        let epoch_secret = &updated_secrets.epoch_secret.0;
        for (_position, key_package) in positions.iter() {
            let group_secret = GroupSecret {
                epoch_secret: SecretVec::new(epoch_secret.expose_secret().to_vec()),
                path_secret: None, // FIXME
            };
            let encrypted_group_secret = self.tree.cs.seal_group_secret(group_secret, key_package); // FIXME: &self.context ?
            secrets.push(encrypted_group_secret);
        }
        Welcome {
            version: PROTOCOL_VERSION_MLS10,
            cipher_suite: self.tree.cs as u16,
            secrets,
            encrypted_group_info,
        }
    }

    fn init_commit(&mut self, add_proposals: &[MLSPlaintext]) -> (MLSPlaintext, Welcome) {
        let add_proposals_ids: Vec<ProposalId> = add_proposals
            .iter()
            .map(|plain| ProposalId(self.tree.cs.hash(&plain.get_encoding())))
            .collect();
        let mut updated_tree = self.tree.clone();
        let positions = updated_tree.update(&add_proposals, &[], &[]);

        // generate init key
        self.owned_kp.update_init_key();

        // update path secrets
        let (path_nodes, parent_hash, commit_secret) = updated_tree.evolve(
            &self.context.get_encoding(),
            self.owned_kp.init_private_key.marshal(),
        );

        // update keypackage's parent_hash extension
        self.owned_kp
            .keypackage
            .payload
            .put_extension(&ext::ParentHashExt(parent_hash));
        self.owned_kp.update_signature();
        updated_tree.set_my_package(self.owned_kp.keypackage.clone());

        let commit = Commit {
            updates: vec![],
            removes: vec![],
            adds: add_proposals_ids,
            path: Some(DirectPath {
                leaf_key_package: self.owned_kp.keypackage.clone(),
                nodes: path_nodes,
            }),
        };
        let updated_epoch = self.context.epoch + 1;
        let confirmed_transcript_hash = self.get_init_confirmed_transcript_hash(&commit);
        let updated_group_context = GroupContext {
            tree_hash: updated_tree.compute_tree_hash(),
            epoch: updated_epoch,
            confirmed_transcript_hash,
            ..self.context.clone()
        };
        let updated_group_context_hash = self.tree.cs.hash(&updated_group_context.get_encoding());
        let epoch_secrets = self.secrets.generate_new_epoch_secrets(
            &commit_secret,
            updated_group_context_hash,
            updated_tree.leaf_len(),
        );
        let confirmation =
            epoch_secrets.compute_confirmation(&updated_group_context.confirmed_transcript_hash);
        let sender = self.get_sender();
        let commit_content = MLSPlaintextCommon {
            group_id: self.context.group_id.clone(),
            epoch: self.context.epoch,
            sender,
            authenticated_data: vec![],
            content: ContentType::Commit {
                commit,
                confirmation: confirmation.clone(),
            },
        };
        let signed_commit = self.get_signed_commit(&commit_content);
        let interim_transcript_hash = self.get_interim_transcript_hash(
            confirmation.clone(),
            signed_commit.signature.clone(),
            updated_group_context.confirmed_transcript_hash.clone(),
        );
        (
            signed_commit,
            self.get_welcome_msg(
                &updated_tree,
                &updated_group_context,
                &epoch_secrets,
                confirmation,
                interim_transcript_hash,
                positions,
            ),
        )
    }

    pub fn process_commit(
        &mut self,
        commit: MLSPlaintext,
        add_proposals: &[MLSPlaintext],
        update_proposals: &[MLSPlaintext],
        remove_proposals: &[MLSPlaintext],
        ra_verifier: &impl AttestedCertVerifier,
        now: Timespec,
    ) -> Result<(), ProcessCommitError> {
        // "Verify that the epoch field of the enclosing MLSPlaintext message
        // is equal to the epoch field of the current GroupContext object"
        if self.context.epoch != commit.content.epoch {
            return Err(ProcessCommitError::GroupEpochError);
        }
        let kp = self
            .tree
            .get_package(LeafSize(commit.content.sender.sender as usize))
            .ok_or(ProcessCommitError::KeyPackageNotFound)?;
        let pk = IdentityPublicKey::new_unsafe(kp.verify(ra_verifier, now)?.public_key);
        // "Verify that the signature on the MLSPlaintext message verifies
        //  using the public key from the credential stored at the leaf in the tree indicated by the sender field."
        commit
            .verify_signature(&self.context, &pk)
            .map_err(kp::Error::SignatureVerifyError)?;

        // "Generate a provisional GroupContext object by applying the proposals referenced in the commit object..."
        for _update in update_proposals.iter() {
            // FIXME
        }
        for _remove in remove_proposals.iter() {
            // FIXME
        }
        let mut add_proposals_ids: BTreeMap<ProposalId, &MLSPlaintext> = BTreeMap::new();
        for add in add_proposals.iter() {
            add_proposals_ids.insert(ProposalId(self.tree.cs.hash(&add.get_encoding())), add);
        }
        let (commit_content, confirmation) = match commit.content.content {
            ContentType::Commit {
                commit,
                confirmation,
            } => Ok((commit, confirmation)),
            _ => Err(ProcessCommitError::CommitError),
        }?;

        let adds = commit_content
            .adds
            .iter()
            .map(|proposal_id| {
                add_proposals_ids
                    .get(proposal_id)
                    .cloned()
                    .cloned()
                    .ok_or(ProcessCommitError::CommitError)
            })
            .collect::<Result<Vec<_>, _>>()?;
        // FIXME: updates
        // FIXME: removes
        let mut updated_tree = self.tree.clone();

        let _ = updated_tree.update(&adds, &[], &[]);
        // "Verify that the path value is populated if either of the updates or removes vectors has length greater than zero
        // all of the updates, removes, and adds vectors are empty."
        if commit_content.path.is_none()
            && ((!commit_content.updates.is_empty() || !commit_content.removes.is_empty())
                || (commit_content.adds.is_empty()
                    && commit_content.updates.is_empty()
                    && commit_content.removes.is_empty()))
        {
            return Err(ProcessCommitError::CommitError);
        }

        let updated_epoch = self.context.epoch + 1;
        let mut updated_group_context = self.context.clone();
        updated_group_context.epoch = updated_epoch;
        // "If the path value is populated: Process the path value..."

        // apply commit path, leaf keypackage and path secrets
        let commit_secret = if let Some(path) = &commit_content.path {
            let sender = LeafSize(commit.content.sender.sender as usize);
            let (leaf_parent_hash, root_secret) = updated_tree.apply_path_secrets(
                sender,
                &self.context.get_encoding(),
                &path.nodes,
                &self.owned_kp.init_private_key,
            )?;
            // Verify that the KeyPackage has a `parent_hash` extension and that its value
            // matches the new parent of the sender's leaf node.
            let ext = path
                .leaf_key_package
                .payload
                .find_extension::<ext::ParentHashExt>()?;
            if !bool::from(ext.0.ct_eq(&leaf_parent_hash)) {
                return Err(ProcessCommitError::LeafParentHashDontMatch);
            }
            updated_tree.set_package(sender, path.leaf_key_package.clone());
            root_secret
        } else {
            // "If the path value is not populated: ..."
            SecretVec::new(vec![0; self.tree.cs.hash_len()])
        };
        updated_group_context.tree_hash = updated_tree.compute_tree_hash();

        // "Update the new GroupContext's confirmed and interim transcript hashes using the new Commit."
        let confirmed_transcript_hash = self.get_init_confirmed_transcript_hash(&commit_content);
        updated_group_context.confirmed_transcript_hash = confirmed_transcript_hash;
        // FIXME: store interim transcript hash?
        let _interim_transcript_hash = self.get_interim_transcript_hash(
            confirmation,
            commit.signature.clone(),
            updated_group_context.confirmed_transcript_hash.clone(),
        );
        let updated_group_context_hash = self.tree.cs.hash(&updated_group_context.get_encoding());

        // "Use the commit_secret, the provisional GroupContext,
        // and the init secret from the previous epoch to compute the epoch secret and derived secrets for the new epoch."
        let epoch_secrets = self.secrets.generate_new_epoch_secrets(
            &commit_secret,
            updated_group_context_hash,
            updated_tree.leaf_len(),
        );
        let confirmation_computed =
            epoch_secrets.compute_confirmation(&updated_group_context.confirmed_transcript_hash);

        // "Use the confirmation_key for the new epoch to compute the confirmation MAC for this message,
        // as described below, and verify that it is the same as the confirmation field in the MLSPlaintext object."
        let confirmation =
            epoch_secrets.compute_confirmation(&updated_group_context.confirmed_transcript_hash);

        let confirmation_ok: bool = confirmation.ct_eq(&confirmation_computed).into();
        if !confirmation_ok {
            return Err(ProcessCommitError::GroupInfoIntegrityError);
        }

        // "If the above checks are successful, consider the updated GroupContext object as the current state of the group."
        self.context = updated_group_context;
        self.secrets = epoch_secrets;
        self.tree = updated_tree;

        Ok(())
    }

    pub fn init_group(
        creator_kp: OwnedKeyPackage,
        others: &[KeyPackage],
        ra_verifier: &impl AttestedCertVerifier,
        genesis_time: Timespec,
    ) -> Result<(Self, Vec<MLSPlaintext>, MLSPlaintext, Welcome), InitGroupError> {
        let mut kps = BTreeSet::new();
        for kp in others.iter() {
            if kps.contains(kp) {
                return Err(InitGroupError::DuplicateKeyPackage);
            } else {
                kp.verify(ra_verifier, genesis_time)?;
                kps.insert(kp.clone());
            }
        }
        if kps.contains(&creator_kp.keypackage) {
            Err(InitGroupError::DuplicateKeyPackage)
        } else {
            creator_kp.keypackage.verify(ra_verifier, genesis_time)?;
            let (context, tree) = GroupContext::init(creator_kp.keypackage.clone());
            let mut group = GroupAux::new(context, tree, creator_kp);
            let add_proposals: Vec<MLSPlaintext> =
                others.iter().map(|kp| group.get_signed_add(kp)).collect();
            let (commit, welcome) = group.init_commit(&add_proposals);
            Ok((group, add_proposals, commit, welcome))
        }
    }

    pub fn init_group_from_welcome(
        my_kp: OwnedKeyPackage,
        welcome: Welcome,
        ra_verifier: &impl AttestedCertVerifier,
        genesis_time: Timespec,
    ) -> Result<Self, ProcessWelcomeError> {
        my_kp.keypackage.verify(ra_verifier, genesis_time)?;
        if welcome.cipher_suite != my_kp.keypackage.payload.cipher_suite {
            return Err(ProcessWelcomeError::CipherSuiteDontMatch);
        }
        if welcome.version != my_kp.keypackage.payload.version {
            return Err(ProcessWelcomeError::VersionDontMatch);
        }
        let cs = match my_kp.keypackage.payload.cipher_suite {
            x if x == (CipherSuite::MLS10_128_DHKEMP256_AES128GCM_SHA256_P256 as u16) => {
                Ok(CipherSuite::MLS10_128_DHKEMP256_AES128GCM_SHA256_P256)
            }
            _ => Err(kp::Error::UnsupportedCipherSuite(
                my_kp.keypackage.payload.cipher_suite,
            )),
        }?;
        let my_kp_hash = cs.hash(&my_kp.keypackage.get_encoding());
        // * "Identify an entry in the secrets array..."
        let msecret = welcome
            .secrets
            .iter()
            .find(|s| s.key_package_hash == my_kp_hash);
        let secret = msecret.ok_or(ProcessWelcomeError::KeyPackageNotFound)?;
        // * "Decrypt the encrypted_group_secrets using HPKE..."
        let group_secret = cs.open_group_secret(&secret, &my_kp);
        let epoch_secret =
            EpochSecrets::<Sha256>::get_epoch_secret(&group_secret.epoch_secret.expose_secret());
        // * "From the epoch_secret in the decrypted GroupSecrets object, derive the welcome_secret, welcome_key, and welcome_nonce..."
        let (welcome_key, welcome_nonce) = EpochSecrets::derive_welcome_secrets(
            &epoch_secret,
            cs.aead_key_len(),
            cs.aead_nonce_len(),
        );
        let group_info =
            cs.open_group_info(&welcome.encrypted_group_info, welcome_key, welcome_nonce);

        // convert Vec<Option<Node>> to Vec<Node>
        let nodes = group_info
            .payload
            .tree
            .clone()
            .into_iter()
            .enumerate()
            .map(|(i, node)| {
                node.unwrap_or_else(|| {
                    if i % 2 == 0 {
                        Node::Leaf(None)
                    } else {
                        Node::Parent(None)
                    }
                })
            })
            .collect::<Vec<_>>();

        // * "Verify the signature on the GroupInfo object..."
        let signer = match nodes.get(group_info.payload.signer_index as usize) {
            Some(Node::Leaf(Some(kp))) => Ok(kp.clone()),
            _ => Err(ProcessWelcomeError::KeyPackageNotFound),
        }?;
        let identity_pk =
            IdentityPublicKey::new_unsafe(signer.verify(ra_verifier, genesis_time)?.public_key);
        let payload = group_info.payload.get_encoding();
        identity_pk
            .verify_signature(&payload, &group_info.signature)
            .map_err(kp::Error::SignatureVerifyError)?;
        // * "Verify the integrity of the ratchet tree..."
        Tree::integrity_check(&nodes, ra_verifier, genesis_time, cs)?;
        // * "Identify a leaf in the tree array..."
        let node_index = nodes
            .iter()
            .enumerate()
            .find(|(_, node)| match node {
                Node::Leaf(Some(kp)) => kp == &my_kp.keypackage,
                _ => false,
            })
            .map(|(i, _)| NodeSize(i))
            .ok_or(ProcessWelcomeError::KeyPackageNotFound)?;
        // * "Construct a new group state using the information in the GroupInfo object..."
        let tree = Tree::from_group_info(
            LeafSize::from_node_index(node_index).expect("invalid leaf index"),
            cs,
            nodes,
        )?;
        if let Some(_path_secret) = group_secret.path_secret {
            // FIXME
        }
        // * "Set the confirmed transcript hash in the new state to the value of the confirmed_transcript_hash in the GroupInfo."
        let context = GroupContext {
            group_id: group_info.payload.group_id.clone(),
            epoch: group_info.payload.epoch,
            tree_hash: tree.compute_tree_hash(),
            confirmed_transcript_hash: group_info.payload.confirmed_transcript_hash.clone(),
            extensions: group_info.payload.extensions,
        };

        // * "Use the epoch_secret from the GroupSecrets object to generate the epoch secret and other derived secrets for the current epoch."
        let secrets = EpochSecrets::from_epoch_secret(
            (group_secret.epoch_secret, epoch_secret),
            tree.cs.hash(&context.get_encoding()),
            tree.leaf_len(),
        );
        let group = GroupAux {
            context,
            tree,
            owned_kp: my_kp,
            secrets,
        };
        // * "Verify the confirmation MAC in the GroupInfo using the derived confirmation key and the confirmed_transcript_hash from the GroupInfo."
        let confirmation = group
            .secrets
            .compute_confirmation(&group.context.confirmed_transcript_hash);

        let confirmation_ok: bool = confirmation.ct_eq(&group_info.payload.confirmation).into();
        if !confirmation_ok {
            return Err(ProcessWelcomeError::GroupInfoIntegrityError);
        }
        Ok(group)
    }
}

const TDBE_GROUP_ID: &[u8] = b"Crypto.com Chain Council Node Transaction Data Bootstrap Enclave";

/// spec: draft-ietf-mls-protocol.md#group-state
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GroupContext {
    /// 0..255 bytes -- application-defined id
    pub group_id: Vec<u8>,
    /// version of the group key
    /// (incremented by 1 for each Commit message
    /// that is processed)
    pub epoch: u64,
    /// commitment to the contents of the
    /// group's ratchet tree and the credentials
    /// for the members of the group
    /// 0..255
    pub tree_hash: Vec<u8>,
    /// field contains a running hash over
    /// the messages that led to this state.
    /// 0..255
    pub confirmed_transcript_hash: Vec<u8>,
    /// 0..2^16-1
    pub extensions: Vec<ext::ExtensionEntry>,
}

impl Codec for GroupContext {
    fn encode(&self, bytes: &mut Vec<u8>) {
        encode_vec_u8_u8(bytes, &self.group_id);
        self.epoch.encode(bytes);
        encode_vec_u8_u8(bytes, &self.tree_hash);
        encode_vec_u8_u8(bytes, &self.confirmed_transcript_hash);
        codec::encode_vec_u16(bytes, &self.extensions);
    }

    fn read(r: &mut Reader) -> Option<Self> {
        let group_id = read_vec_u8_u8(r)?;
        let epoch = u64::read(r)?;
        let tree_hash = read_vec_u8_u8(r)?;
        let confirmed_transcript_hash = read_vec_u8_u8(r)?;
        let extensions = codec::read_vec_u16(r)?;
        Some(Self {
            group_id,
            epoch,
            tree_hash,
            confirmed_transcript_hash,
            extensions,
        })
    }
}

impl GroupContext {
    pub fn init(creator_kp: KeyPackage) -> (Self, Tree) {
        let extensions = creator_kp.payload.extensions.clone();
        let tree = Tree::init(creator_kp);
        (
            GroupContext {
                group_id: TDBE_GROUP_ID.to_vec(),
                epoch: 0,
                tree_hash: tree.compute_tree_hash(),
                confirmed_transcript_hash: vec![],
                extensions,
            },
            tree,
        )
    }
}

/// spec: draft-ietf-mls-protocol.md#Welcoming-New-Members
#[derive(Debug, Clone)]
pub struct GroupInfoPayload {
    /// 0..255 bytes -- application-defined id
    pub group_id: Vec<u8>,
    /// version of the group key
    /// (incremented by 1 for each Commit message
    /// that is processed)
    pub epoch: u64,
    /// 1..2^32-1
    /// FIXME representation may change https://github.com/mlswg/mls-protocol/issues/344
    pub tree: Vec<Option<Node>>,
    /// 0..255
    pub confirmed_transcript_hash: Vec<u8>,
    /// 0..255
    pub interim_transcript_hash: Vec<u8>,
    /// 0..2^16-1
    pub extensions: Vec<ext::ExtensionEntry>,
    /// 0..255
    pub confirmation: Vec<u8>,
    pub signer_index: u32,
}

impl Codec for GroupInfoPayload {
    fn encode(&self, bytes: &mut Vec<u8>) {
        encode_vec_u8_u8(bytes, &self.group_id);
        self.epoch.encode(bytes);
        encode_vec_option_u32(bytes, &self.tree);
        encode_vec_u8_u8(bytes, &self.confirmed_transcript_hash);
        encode_vec_u8_u8(bytes, &self.interim_transcript_hash);
        codec::encode_vec_u16(bytes, &self.extensions);
        encode_vec_u8_u8(bytes, &self.confirmation);
        self.signer_index.encode(bytes);
    }

    fn read(r: &mut Reader) -> Option<Self> {
        let group_id = read_vec_u8_u8(r)?;
        let epoch = u64::read(r)?;
        let tree = read_vec_option_u32(r)?;
        let confirmed_transcript_hash = read_vec_u8_u8(r)?;
        let interim_transcript_hash = read_vec_u8_u8(r)?;
        let extensions = codec::read_vec_u16(r)?;
        let confirmation = read_vec_u8_u8(r)?;
        let signer_index = u32::read(r)?;
        Some(GroupInfoPayload {
            group_id,
            epoch,
            tree,
            confirmed_transcript_hash,
            interim_transcript_hash,
            extensions,
            confirmation,
            signer_index,
        })
    }
}

/// spec: draft-ietf-mls-protocol.md#Welcoming-New-Members
#[derive(Debug, Clone)]
pub struct GroupInfo {
    pub payload: GroupInfoPayload,
    // 0..2^16-1
    pub signature: Vec<u8>,
}

impl Codec for GroupInfo {
    fn encode(&self, bytes: &mut Vec<u8>) {
        self.payload.encode(bytes);
        encode_vec_u8_u16(bytes, &self.signature);
    }

    fn read(r: &mut Reader) -> Option<Self> {
        let payload = GroupInfoPayload::read(r)?;
        let signature = read_vec_u8_u16(r)?;
        Some(GroupInfo { payload, signature })
    }
}

#[derive(thiserror::Error, Debug)]
pub enum ProcessCommitError {
    #[error("keypackage verify failed: {0}")]
    KeyPackageVerifyFail(#[from] kp::Error),
    #[error("find extension failed: {0}")]
    FindExtensionFail(#[from] kp::FindExtensionError),
    #[error("Epoch does not match")]
    GroupEpochError,
    #[error("group info integrity check failed")]
    GroupInfoIntegrityError,
    #[error("ratchet tree integrity check failed: {0}")]
    TreeVerifyFail(#[from] TreeIntegrityError),
    #[error("Commit processing error")]
    CommitError,
    #[error("key package not found")]
    KeyPackageNotFound,
    #[error("decrypted path secret don't match the public key")]
    PathSecretPublicKeyDontMatch,
    #[error("parent hash extension in leaf keypackage don't match")]
    LeafParentHashDontMatch,
}

#[derive(thiserror::Error, Debug)]
pub enum ProcessWelcomeError {
    #[error("keypackage verify failed: {0}")]
    KeyPackageVerifyFail(#[from] kp::Error),
    #[error("ratchet tree integrity check failed: {0}")]
    TreeVerifyFail(#[from] TreeIntegrityError),
    #[error("key package not found")]
    KeyPackageNotFound,
    #[error("cipher suite in welcome don't match keypackage")]
    CipherSuiteDontMatch,
    #[error("version in welcome don't match keypackage")]
    VersionDontMatch,
    #[error("group info integrity check failed")]
    GroupInfoIntegrityError,
}

#[derive(thiserror::Error, Debug)]
pub enum InitGroupError {
    #[error("keypackage verify failed: {0}")]
    KeyPackageVerifyFail(#[from] kp::Error),
    #[error("duplicate keypackages")]
    DuplicateKeyPackage,
}

#[cfg(test)]
mod test {

    use super::*;
    use crate::credential::Credential;
    use crate::extensions::{self as ext, MLSExtension};
    use crate::key::{HPKEPrivateKey, IdentityPrivateKey};
    use crate::keypackage::{
        KeyPackage, KeyPackagePayload, OwnedKeyPackage, MLS10_128_DHKEMP256_AES128GCM_SHA256_P256,
        PROTOCOL_VERSION_MLS10,
    };
    use chrono::{DateTime, Utc};
    use ra_client::ENCLAVE_CERT_VERIFIER;
    use ra_client::{AttestedCertVerifier, CertVerifyResult, EnclaveCertVerifierError};
    use rustls::internal::msgs::codec::Codec;

    #[derive(Clone)]
    struct MockVerifier();

    impl AttestedCertVerifier for MockVerifier {
        fn verify_attested_cert(
            &self,
            certificate: &[u8],
            _now: DateTime<Utc>,
        ) -> Result<CertVerifyResult, EnclaveCertVerifierError> {
            static VECTOR: &[u8] = include_bytes!("../tests/test_vectors/keypackage.bin");
            let kp = <KeyPackage>::read_bytes(VECTOR).expect("decode");
            let now = 1590490084;
            let t = kp.verify(&*ENCLAVE_CERT_VERIFIER, now).unwrap();

            Ok(CertVerifyResult {
                public_key: certificate.to_vec(),
                quote: t.quote,
            })
        }
    }

    fn get_fake_keypackage() -> OwnedKeyPackage {
        let keypair = ring::signature::EcdsaKeyPair::generate_pkcs8(
            &ring::signature::ECDSA_P256_SHA256_ASN1_SIGNING,
            &ring::rand::SystemRandom::new(),
        )
        .unwrap();
        let extensions = vec![
            ext::SupportedVersionsExt(vec![PROTOCOL_VERSION_MLS10]).entry(),
            ext::SupportedCipherSuitesExt(vec![MLS10_128_DHKEMP256_AES128GCM_SHA256_P256]).entry(),
            ext::LifeTimeExt::new(0, 100).entry(),
        ];

        let private_key =
            IdentityPrivateKey::from_pkcs8(keypair.as_ref()).expect("invalid private key");
        let (hpke_secret, hpke_public) = HPKEPrivateKey::generate();

        let payload = KeyPackagePayload {
            version: PROTOCOL_VERSION_MLS10,
            cipher_suite: MLS10_128_DHKEMP256_AES128GCM_SHA256_P256,
            init_key: hpke_public,
            credential: Credential::X509(private_key.public_key_raw().to_vec()),
            extensions,
        };

        // sign payload
        let signature = private_key.sign(&payload.get_encoding());

        OwnedKeyPackage {
            keypackage: KeyPackage { payload, signature },
            credential_private_key: private_key,
            init_private_key: hpke_secret,
        }
    }

    #[test]
    fn test_sign_verify_add() {
        let creator_kp = get_fake_keypackage();
        let to_be_added = get_fake_keypackage().keypackage;
        let (context, tree) = GroupContext::init(creator_kp.keypackage.clone());
        let group_aux = GroupAux::new(context, tree, creator_kp);
        let plain = group_aux.get_signed_add(&to_be_added);
        assert!(plain
            .verify_signature(
                &group_aux.context,
                &group_aux.owned_kp.credential_private_key.public_key()
            )
            .is_ok());
    }

    #[test]
    fn test_welcome_commit_process() {
        let creator_kp = get_fake_keypackage();
        let to_be_added = get_fake_keypackage();
        let (mut creator_group, adds, commit, welcome) = GroupAux::init_group(
            creator_kp,
            &[to_be_added.keypackage.clone()],
            &MockVerifier {},
            0,
        )
        .expect("group init");
        let added_group =
            GroupAux::init_group_from_welcome(to_be_added, welcome, &MockVerifier {}, 0)
                .expect("group init from welcome");
        creator_group
            .process_commit(commit, &adds, &[], &[], &MockVerifier {}, 0)
            .expect("commit ok");
        // they should get to the same context
        assert_eq!(&added_group.context, &creator_group.context);
    }
}
