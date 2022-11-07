/* This file is part of DarkFi (https://dark.fi)
 *
 * Copyright (C) 2020-2022 Dyne.org foundation
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as
 * published by the Free Software Foundation, either version 3 of the
 * License, or (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU Affero General Public License for more details.
 *
 * You should have received a copy of the GNU Affero General Public License
 * along with this program.  If not, see <https://www.gnu.org/licenses/>.
 */

use std::time::Instant;

use darkfi_sdk::{
    crypto::{
        pedersen::{pedersen_commitment_base, pedersen_commitment_u64},
        MerkleNode, Nullifier, PublicKey, SecretKey,
    },
    incrementalmerkletree::Hashable,
    pasta::{arithmetic::CurveAffine, group::Curve},
};
use darkfi_serial::{SerialDecodable, SerialEncodable};
use halo2_proofs::circuit::Value;
use log::debug;
use rand::rngs::OsRng;

use super::proof::{Proof, ProvingKey, VerifyingKey};
use crate::{
    crypto::{
        types::{
            DrkCircuitField, DrkCoinBlind, DrkSerial, DrkSpendHook, DrkTokenId, DrkUserData,
            DrkUserDataBlind, DrkUserDataEnc, DrkValue, DrkValueBlind, DrkValueCommit,
        },
        util::poseidon_hash,
    },
    zk::circuit::burn_contract::BurnContract,
    Result,
};

#[derive(Debug, Clone, PartialEq, Eq, SerialEncodable, SerialDecodable)]
pub struct BurnRevealedValues {
    pub value_commit: DrkValueCommit,
    pub token_commit: DrkValueCommit,
    pub nullifier: Nullifier,
    pub merkle_root: MerkleNode,
    pub spend_hook: DrkSpendHook,
    pub user_data_enc: DrkUserDataEnc,
    pub signature_public: PublicKey,
}

impl BurnRevealedValues {
    #[allow(clippy::too_many_arguments)]
    pub fn compute(
        value: u64,
        token_id: DrkTokenId,
        value_blind: DrkValueBlind,
        token_blind: DrkValueBlind,
        serial: DrkSerial,
        coin_blind: DrkCoinBlind,
        secret: SecretKey,
        leaf_position: incrementalmerkletree::Position,
        merkle_path: Vec<MerkleNode>,
        spend_hook: DrkSpendHook,
        user_data: DrkUserData,
        user_data_blind: DrkUserDataBlind,
        signature_secret: SecretKey,
    ) -> Self {
        let nullifier = Nullifier::from(poseidon_hash::<2>([secret.inner(), serial]));

        let public_key = PublicKey::from_secret(secret);
        let (pub_x, pub_y) = public_key.xy();

        let coin = poseidon_hash::<8>([
            pub_x,
            pub_y,
            DrkValue::from(value),
            token_id,
            serial,
            spend_hook,
            user_data,
            coin_blind,
        ]);

        let merkle_root = {
            let position: u64 = leaf_position.into();
            let mut current = MerkleNode::from(coin);
            for (level, sibling) in merkle_path.iter().enumerate() {
                let level = level as u8;
                current = if position & (1 << level) == 0 {
                    MerkleNode::combine(level.into(), &current, sibling)
                } else {
                    MerkleNode::combine(level.into(), sibling, &current)
                };
            }
            current
        };

        let user_data_enc = poseidon_hash::<2>([user_data, user_data_blind]);

        let value_commit = pedersen_commitment_u64(value, value_blind);
        let token_commit = pedersen_commitment_base(token_id, token_blind);

        BurnRevealedValues {
            value_commit,
            token_commit,
            nullifier,
            merkle_root,
            spend_hook,
            user_data_enc,
            signature_public: PublicKey::from_secret(signature_secret),
        }
    }

    pub fn make_outputs(&self) -> Vec<DrkCircuitField> {
        let value_coords = self.value_commit.to_affine().coordinates().unwrap();
        let token_coords = self.token_commit.to_affine().coordinates().unwrap();
        let merkle_root = self.merkle_root.inner();
        let user_data_enc = self.user_data_enc;
        let (sig_x, sig_y) = self.signature_public.xy();

        vec![
            self.nullifier.inner(),
            *value_coords.x(),
            *value_coords.y(),
            *token_coords.x(),
            *token_coords.y(),
            merkle_root,
            user_data_enc,
            sig_x,
            sig_y,
        ]
    }
}

#[allow(clippy::too_many_arguments)]
pub fn create_burn_proof(
    pk: &ProvingKey,
    value: u64,
    token_id: DrkTokenId,
    value_blind: DrkValueBlind,
    token_blind: DrkValueBlind,
    serial: DrkSerial,
    spend_hook: DrkSpendHook,
    user_data: DrkUserData,
    user_data_blind: DrkUserDataBlind,
    coin_blind: DrkCoinBlind,
    secret: SecretKey,
    leaf_position: incrementalmerkletree::Position,
    merkle_path: Vec<MerkleNode>,
    signature_secret: SecretKey,
) -> Result<(Proof, BurnRevealedValues)> {
    let revealed = BurnRevealedValues::compute(
        value,
        token_id,
        value_blind,
        token_blind,
        serial,
        coin_blind,
        secret,
        leaf_position,
        merkle_path.clone(),
        spend_hook,
        user_data,
        user_data_blind,
        signature_secret,
    );

    let leaf_position: u64 = leaf_position.into();

    let c = BurnContract {
        secret_key: Value::known(secret.inner()),
        serial: Value::known(serial),
        value: Value::known(DrkValue::from(value)),
        token: Value::known(token_id),
        coin_blind: Value::known(coin_blind),
        value_blind: Value::known(value_blind),
        token_blind: Value::known(token_blind),
        leaf_pos: Value::known(leaf_position as u32),
        merkle_path: Value::known(merkle_path.try_into().unwrap()),
        spend_hook: Value::known(spend_hook),
        user_data: Value::known(user_data),
        user_data_blind: Value::known(user_data_blind),
        sig_secret: Value::known(signature_secret.inner()),
    };

    let start = Instant::now();
    let public_inputs = revealed.make_outputs();
    let proof = Proof::create(pk, &[c], &public_inputs, &mut OsRng)?;
    debug!("Prove burn: [{:?}]", start.elapsed());

    Ok((proof, revealed))
}

pub fn verify_burn_proof(
    vk: &VerifyingKey,
    proof: &Proof,
    revealed: &BurnRevealedValues,
) -> Result<()> {
    let start = Instant::now();
    let public_inputs = revealed.make_outputs();
    proof.verify(vk, &public_inputs)?;
    debug!("Verify burn: [{:?}]", start.elapsed());
    Ok(())
}
