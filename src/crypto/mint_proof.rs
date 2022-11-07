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
        PublicKey,
    },
    pasta::{arithmetic::CurveAffine, group::Curve},
};
use darkfi_serial::{SerialDecodable, SerialEncodable};
use halo2_proofs::circuit::Value;
use log::debug;
use rand::rngs::OsRng;

use crate::{
    crypto::{
        coin::Coin,
        proof::{Proof, ProvingKey, VerifyingKey},
        types::{
            DrkCircuitField, DrkCoinBlind, DrkSerial, DrkSpendHook, DrkTokenId, DrkUserData,
            DrkValue, DrkValueBlind, DrkValueCommit,
        },
        util::poseidon_hash,
    },
    zk::circuit::mint_contract::MintContract,
    Result,
};

#[derive(Debug, Clone, PartialEq, Eq, SerialEncodable, SerialDecodable)]
pub struct MintRevealedValues {
    pub value_commit: DrkValueCommit,
    pub token_commit: DrkValueCommit,
    pub coin: Coin,
}

impl MintRevealedValues {
    #[allow(clippy::too_many_arguments)]
    pub fn compute(
        value: u64,
        token_id: DrkTokenId,
        value_blind: DrkValueBlind,
        token_blind: DrkValueBlind,
        serial: DrkSerial,
        spend_hook: DrkSpendHook,
        user_data: DrkUserData,
        coin_blind: DrkCoinBlind,
        public_key: PublicKey,
    ) -> Self {
        let value_commit = pedersen_commitment_u64(value, value_blind);
        let token_commit = pedersen_commitment_base(token_id, token_blind);

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

        MintRevealedValues { value_commit, token_commit, coin: Coin(coin) }
    }

    pub fn make_outputs(&self) -> Vec<DrkCircuitField> {
        let value_coords = self.value_commit.to_affine().coordinates().unwrap();
        let token_coords = self.token_commit.to_affine().coordinates().unwrap();

        vec![
            self.coin.0,
            *value_coords.x(),
            *value_coords.y(),
            *token_coords.x(),
            *token_coords.y(),
        ]
    }
}

#[allow(clippy::too_many_arguments)]
pub fn create_mint_proof(
    pk: &ProvingKey,
    value: u64,
    token_id: DrkTokenId,
    value_blind: DrkValueBlind,
    token_blind: DrkValueBlind,
    serial: DrkSerial,
    spend_hook: DrkSpendHook,
    user_data: DrkUserData,
    coin_blind: DrkCoinBlind,
    public_key: PublicKey,
) -> Result<(Proof, MintRevealedValues)> {
    let revealed = MintRevealedValues::compute(
        value,
        token_id,
        value_blind,
        token_blind,
        serial,
        spend_hook,
        user_data,
        coin_blind,
        public_key,
    );

    let (pub_x, pub_y) = public_key.xy();

    let c = MintContract {
        pub_x: Value::known(pub_x),
        pub_y: Value::known(pub_y),
        value: Value::known(DrkValue::from(value)),
        token: Value::known(token_id),
        serial: Value::known(serial),
        coin_blind: Value::known(coin_blind),
        spend_hook: Value::known(spend_hook),
        user_data: Value::known(user_data),
        value_blind: Value::known(value_blind),
        token_blind: Value::known(token_blind),
    };

    let start = Instant::now();
    let public_inputs = revealed.make_outputs();
    let proof = Proof::create(pk, &[c], &public_inputs, &mut OsRng)?;
    debug!("Prove mint: [{:?}]", start.elapsed());

    Ok((proof, revealed))
}

pub fn verify_mint_proof(
    vk: &VerifyingKey,
    proof: &Proof,
    revealed: &MintRevealedValues,
) -> Result<()> {
    let start = Instant::now();
    let public_inputs = revealed.make_outputs();
    proof.verify(vk, &public_inputs)?;
    debug!("Verify mint: [{:?}]", start.elapsed());
    Ok(())
}
