use halo2_base::halo2_proofs::{
    circuit::{AssignedCell, Layouter, Region, SimpleFloorPlanner, Value},
    plonk::{
        Advice, Assigned, Circuit, Column, ConstraintSystem, Constraints, Error, Expression,
        Instance, Selector,
    },
    poly::Rotation,
};
use halo2_base::{
    gates::{flex_gate::FlexGateConfig, range::RangeConfig, GateInstructions, RangeInstructions},
    utils::{bigint_to_fe, biguint_to_fe, fe_to_biguint, modulus, PrimeField},
    AssignedValue, Context, QuantumCell,
};
use std::marker::PhantomData;

pub use crate::table::TransitionTableConfig;

#[derive(Debug, Clone)]
struct RangeConstrained<F: PrimeField>(AssignedCell<F, F>);

#[derive(Debug, Clone)]
pub struct AssignedRegexResult<F: PrimeField> {
    pub characters: Vec<AssignedCell<F, F>>,
    pub states: Vec<AssignedCell<F, F>>,
}

// Here we decompose a transition into 3-value lookups.

#[derive(Debug, Clone)]
pub struct RegexCheckConfig<F: PrimeField> {
    characters: Column<Advice>,
    // characters_advice: Column<Instance>,
    state: Column<Advice>,
    transition_table: TransitionTableConfig<F>,
    q_lookup_state_selector: Selector,
    _marker: PhantomData<F>,
}

impl<F: PrimeField> RegexCheckConfig<F> {
    pub fn configure(meta: &mut ConstraintSystem<F>) -> Self {
        let characters = meta.advice_column();
        let state = meta.advice_column();
        let q_lookup_state_selector = meta.complex_selector();
        let transition_table = TransitionTableConfig::configure(meta);

        // Lookup each transition value individually, not paying attention to bit count
        meta.lookup("lookup characters and their state", |meta| {
            let q = meta.query_selector(q_lookup_state_selector);
            let prev_state = meta.query_advice(state, Rotation::cur());
            let next_state = meta.query_advice(state, Rotation::next());
            let character = meta.query_advice(characters, Rotation::cur());

            // One minus q
            let one_minus_q = Expression::Constant(F::from(1)) - q.clone();
            let zero = Expression::Constant(F::from(0));

            /*
                | q | state | characters | table.prev_state | table.next_state  | table.character
                | 1 | s_cur |    char    |       s_cur      |     s_next        |     char
                |   | s_next|
            */

            vec![
                (
                    q.clone() * prev_state + one_minus_q.clone() * zero.clone(),
                    transition_table.prev_state,
                ),
                (
                    q.clone() * next_state + one_minus_q.clone() * zero.clone(),
                    transition_table.next_state,
                ),
                (
                    q.clone() * character + one_minus_q.clone() * zero.clone(),
                    transition_table.character,
                ),
            ]
        });

        Self {
            characters,
            state,
            q_lookup_state_selector,
            transition_table,
            _marker: PhantomData,
        }
    }

    pub fn load(
        &self,
        layouter: &mut impl Layouter<F>,
        lookup_filepath: &str,
    ) -> Result<(), Error> {
        self.transition_table.load(layouter, lookup_filepath)
    }

    // Note that the two types of region.assign_advice calls happen together so that it is the same region
    pub fn assign_values(
        &self,
        region: &mut Region<F>,
        characters: &[u8],
        states: &[u64],
    ) -> Result<AssignedRegexResult<F>, Error> {
        let mut assigned_characters = Vec::new();
        let mut assigned_states = Vec::new();
        debug_assert_eq!(characters.len(), states.len());
        // layouter.assign_region(
        //     || "Assign values",
        //     |mut region| {
        //         // let offset = 0;

        //         // Enable q_decomposed
        //         for i in 0..STRING_LEN {
        //             println!("{:?}, {:?}", characters[i], states[i]);
        //             // offset = i;
        //             if i < STRING_LEN - 1 {
        //                 self.q_lookup_state_selector.enable(&mut region, i)?;
        //             }
        //             let assigned_c = region.assign_advice(
        //                 || format!("character"),
        //                 self.characters,
        //                 i,
        //                 || Value::known(F::from(characters[i] as u64)),
        //             )?;
        //             assigned_characters.push(assigned_c);
        //             let assigned_s = region.assign_advice(
        //                 || format!("state"),
        //                 self.state,
        //                 i,
        //                 || Value::known(F::from_u128(states[i])),
        //             )?;
        //             assigned_states.push(assigned_s);
        //         }
        //         Ok(())
        //     },
        // )?;
        // Enable q_decomposed
        for i in 0..STRING_LEN {
            println!("{:?}, {:?}", characters[i], states[i]);
            // offset = i;
            if i < STRING_LEN - 1 {
                self.q_lookup_state_selector.enable(region, i)?;
            }
            let assigned_c = region.assign_advice(
                || format!("character"),
                self.characters,
                i,
                || Value::known(F::from(characters[i] as u64)),
            )?;
            assigned_characters.push(assigned_c);
            let assigned_s = region.assign_advice(
                || format!("state"),
                self.state,
                i,
                || Value::known(F::from(states[i])),
            )?;
            assigned_states.push(assigned_s);
        }
        Ok(AssignedRegexResult {
            characters: assigned_characters,
            states: assigned_states,
        })
    }
}

#[cfg(test)]
mod tests {
    use halo2_base::halo2_proofs::{
        circuit::floor_planner::V1,
        dev::{FailureLocation, MockProver, VerifyFailure},
        halo2curves::bn256::Fr,
        plonk::{Any, Circuit},
    };

    use super::*;

    // Checks a regex of string len
    const STRING_LEN: usize = 22;

    #[derive(Default, Clone)]
    struct TestRegexCheckCircuit<F: PrimeField> {
        // Since this is only relevant for the witness, we can opt to make this whatever convenient type we want
        pub characters: Vec<u8>,
        pub states: Vec<u64>,
        _marker: PhantomData<F>,
    }

    impl<F: PrimeField> Circuit<F> for TestRegexCheckCircuit<F> {
        type Config = RegexCheckConfig<F>;
        type FloorPlanner = SimpleFloorPlanner;

        // Circuit without witnesses, called only during key generation
        fn without_witnesses(&self) -> Self {
            Self {
                characters: vec![],
                states: vec![],
                _marker: PhantomData,
            }
        }

        fn configure(meta: &mut ConstraintSystem<F>) -> Self::Config {
            let config = RegexCheckConfig::configure(meta);
            config
        }

        fn synthesize(
            &self,
            config: Self::Config,
            mut layouter: impl Layouter<F>,
        ) -> Result<(), Error> {
            // test regex: "email was meant for @(a|b|c|d|e|f|g|h|i|j|k|l|m|n|o|p|q|r|s|t|u|v|w|x|y|z|A|B|C|D|E|F|G|H|I|J|K|L|M|N|O|P|Q|R|S|T|U|V|W|X|Y|Z|0|1|2|3|4|5|6|7|8|9|_)+"
            config.load(&mut layouter, "./test_regexes/regex_test_lookup.txt")?;
            print!("Synthesize being called...");
            layouter.assign_region(
                || "regex",
                |mut region| {
                    config.assign_values(&mut region, &self.characters, &self.states)?;
                    Ok(())
                },
            )?;
            Ok(())
        }
    }

    #[test]
    fn test_regex_pass() {
        let k = 7; // 8, 128, etc

        // Convert query string to u128s
        let characters: Vec<u8> = "email was meant for @y".chars().map(|c| c as u8).collect();

        // Make a vector of the numbers 1...24
        let states = (1..=STRING_LEN as u64).collect::<Vec<u64>>();
        assert_eq!(characters.len(), STRING_LEN);
        assert_eq!(states.len(), STRING_LEN);

        // Successful cases
        let circuit = TestRegexCheckCircuit::<Fr> {
            characters,
            states,
            _marker: PhantomData,
        };

        let prover = MockProver::run(k, &circuit, vec![]).unwrap();
        prover.assert_satisfied();
    }

    #[test]
    fn test_regex_fail() {
        let k = 10;

        // Convert query string to u128s
        let characters: Vec<u8> = "email isnt meant for u".chars().map(|c| c as u8).collect();

        // Make a vector of the numbers 1...24
        let states = (1..=STRING_LEN as u64).collect::<Vec<u64>>();

        assert_eq!(characters.len(), STRING_LEN);
        assert_eq!(states.len(), STRING_LEN);

        // Out-of-range `value = 8`
        let circuit = TestRegexCheckCircuit::<Fr> {
            characters: characters,
            states: states,
            _marker: PhantomData,
        };
        let prover = MockProver::run(k, &circuit, vec![]).unwrap();
        match prover.verify() {
            Err(e) => {
                println!("Error successfully achieved!");
            }
            _ => assert_eq!(1, 0),
        }
    }

    // $ cargo test --release --all-features print_range_check_1
    #[cfg(feature = "dev-graph")]
    #[test]
    fn print_range_check_1() {
        use plotters::prelude::*;

        let root = BitMapBackend::new("range-check-decomposed-layout.png", (1024, 3096))
            .into_drawing_area();
        root.fill(&WHITE).unwrap();
        let root = root
            .titled("Range Check 1 Layout", ("sans-serif", 60))
            .unwrap();

        let circuit = RegexCheckCircuit::<Fp> {
            value: 2 as u128,
            _marker: PhantomData,
        };
        halo2_proofs::dev::CircuitLayout::default()
            .render(3, &circuit, &root)
            .unwrap();
    }
}
