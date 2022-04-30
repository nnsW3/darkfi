use std::{marker::PhantomData, ops::Deref};

use halo2_proofs::{
    arithmetic::FieldExt,
    circuit::{AssignedCell, Chip, Layouter, Region},
    plonk::{Advice, Column, ConstraintSystem, Error, Expression, Selector, TableColumn},
    poly::Rotation,
};

/// Chip state is stored in a config struct. This is generated by the
/// chip during configuration, and then stored inside the chip
#[derive(Clone, Debug)]
pub struct EvenBitsConfig {
    advice: [Column<Advice>; 2],
    even_bits: TableColumn,

    s_decompose: Selector,
}

impl EvenBitsConfig {
    pub fn load_private<F: FieldExt>(
        &self,
        mut layouter: impl Layouter<F>,
        value: Option<F>,
    ) -> Result<AssignedCell<F, F>, Error> {
        layouter.assign_region(
            || "load private",
            |mut region| {
                region.assign_advice(
                    || "private input",
                    self.advice[0],
                    0,
                    || value.ok_or(Error::Synthesis),
                )
            },
        )
    }
}

#[derive(Clone, Debug)]
pub struct EvenBitsChip<F: FieldExt, const WORD_BITS: u32> {
    config: EvenBitsConfig,
    _marker: PhantomData<F>,
}

impl<F: FieldExt, const WORD_BITS: u32> Chip<F> for EvenBitsChip<F, WORD_BITS> {
    type Config = EvenBitsConfig;
    type Loaded = ();

    fn config(&self) -> &Self::Config {
        &self.config
    }

    fn loaded(&self) -> &Self::Loaded {
        &()
    }
}

impl<F: FieldExt, const WORD_BITS: u32> EvenBitsChip<F, WORD_BITS> {
    pub fn construct(config: <Self as Chip<F>>::Config) -> Self {
        Self { config, _marker: PhantomData }
    }

    pub fn configure(meta: &mut ConstraintSystem<F>) -> <Self as Chip<F>>::Config {
        let advice = [meta.advice_column(), meta.advice_column()];
        for column in &advice {
            meta.enable_equality(*column);
        }

        let s_decompose = meta.complex_selector();
        let even_bits = meta.lookup_table_column();

        meta.create_gate("decompose", |meta| {
            let lhs = meta.query_advice(advice[0], Rotation::cur());
            let rhs = meta.query_advice(advice[1], Rotation::cur());
            let out = meta.query_advice(advice[0], Rotation::next());
            let s_decompose = meta.query_selector(s_decompose);

            // Finally, we return the polynomial expressions that constrain this gate.
            // For our multiplication gate, we only need a single polynomial constraint.
            //
            // The polynomial expressions returned from `create_gate` will be
            // constrained by the proving system to equal zero.
            vec![s_decompose * (lhs + Expression::Constant(F::from(2)) * rhs - out)]
        });

        let _ = meta.lookup(|meta| {
            let lookup = meta.query_selector(s_decompose);
            let a = meta.query_advice(advice[0], Rotation::cur());

            vec![(lookup * a, even_bits)]
        });

        let _ = meta.lookup(|meta| {
            let lookup = meta.query_selector(s_decompose);
            let b = meta.query_advice(advice[1], Rotation::cur());

            vec![(lookup * b, even_bits)]
        });

        EvenBitsConfig { advice, even_bits, s_decompose }
    }

    // Allocates all even bits in a table for the word size WORD_BITS.
    // `2^(WORD_BITS/2)` rows of the constraint system
    pub fn alloc_table(&self, layouter: &mut impl Layouter<F>) -> Result<(), Error> {
        layouter.assign_table(
            || "even bits table",
            |mut table| {
                for i in 0..2usize.pow(WORD_BITS / 2) {
                    table.assign_cell(
                        || format!("even_bits row {}", i),
                        self.config.even_bits,
                        i,
                        || Ok(F::from(even_bits_at(i) as u64)),
                    )?;
                }
                Ok(())
            },
        )
    }
}

fn even_bits_at(mut i: usize) -> usize {
    let mut r = 0;
    let mut c = 0;

    while i != 0 {
        let lower_bit = i % 2;
        r += lower_bit * 4usize.pow(c);
        i >>= 1;
        c += 1;
    }

    r
}

/// A newtype of a field element containing only bits that were in the
/// even position of the decomposed element.
/// All odd bits will be zero.
#[derive(Clone, Copy, Debug)]
pub struct EvenBits<W>(pub W);

impl<W> Deref for EvenBits<W> {
    type Target = W;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// A newtype of a field element containing only bits thet were in the
/// odd position of the decomposed element.
/// All odd bits will be right shifted by 1 into even positions.
/// All odd bits will be zero.
#[derive(Clone, Copy, Debug)]
pub struct OddBits<W>(pub W);

impl<W> Deref for OddBits<W> {
    type Target = W;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub trait EvenBitsLookup<F: FieldExt>: Chip<F> {
    type Word;

    #[allow(clippy::type_complexity)]
    fn decompose(
        &self,
        layouter: impl Layouter<F>,
        c: Self::Word,
    ) -> Result<(EvenBits<Self::Word>, OddBits<Self::Word>), Error>;
}

impl<F: FieldExt, const WORD_BITS: u32> EvenBitsLookup<F> for EvenBitsChip<F, WORD_BITS> {
    type Word = AssignedCell<F, F>;

    fn decompose(
        &self,
        mut layouter: impl Layouter<F>,
        c: Self::Word,
    ) -> Result<(EvenBits<Self::Word>, OddBits<Self::Word>), Error> {
        let config = self.config();

        layouter.assign_region(
            || "decompose",
            |mut region: Region<'_, F>| {
                config.s_decompose.enable(&mut region, 0)?;

                let o_eo = c.value().cloned().map(decompose);
                let e_cell = region
                    .assign_advice(
                        || "even bits",
                        config.advice[0],
                        0,
                        || o_eo.map(|eo| *eo.0).ok_or(Error::Synthesis),
                    )
                    .map(EvenBits)?;

                let o_cell = region
                    .assign_advice(
                        || "odd bits",
                        config.advice[1],
                        0,
                        || o_eo.map(|eo| *eo.1).ok_or(Error::Synthesis),
                    )
                    .map(OddBits)?;

                c.copy_advice(|| "out", &mut region, config.advice[0], 1)?;
                Ok((e_cell, o_cell))
            },
        )
    }
}

fn decompose<F: FieldExt>(word: F) -> (EvenBits<F>, OddBits<F>) {
    assert!(word <= F::from_u128(u128::MAX));

    let mut even_only = word.to_repr();
    even_only.as_mut().iter_mut().for_each(|bits| {
        *bits &= 0b01010101;
    });

    let mut odd_only = word.to_repr();
    odd_only.as_mut().iter_mut().for_each(|bits| {
        *bits &= 0b10101010;
    });

    let even_only = EvenBits(F::from_repr(even_only).unwrap());
    let odd_only = F::from_repr(odd_only).unwrap();
    let odds_in_even = OddBits(F::from_u128(odd_only.get_lower_128() >> 1));
    (even_only, odds_in_even)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn even_bits_at_test() {
        assert_eq!(0b0, even_bits_at(0));
        assert_eq!(0b1, even_bits_at(1));
        assert_eq!(0b100, even_bits_at(2));
        assert_eq!(0b101, even_bits_at(3));
    }

    #[test]
    fn decompose_even_odd_test() {
        use pasta_curves::pallas;
        let odds = 0xAAAA;
        let evens = 0x5555;
        let (e, o) = decompose(pallas::Base::from_u128(odds));
        assert_eq!(e.get_lower_128(), 0);
        assert_eq!(o.get_lower_128(), odds >> 1);
        let (e, o) = decompose(pallas::Base::from_u128(evens));
        assert_eq!(e.get_lower_128(), evens);
        assert_eq!(o.get_lower_128(), 0);
    }
}
//
