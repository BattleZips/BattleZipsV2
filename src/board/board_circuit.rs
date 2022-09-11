use crate::board::board_chip::BoardConfig;
use halo2_proofs::{arithmetic::FieldExt, circuit::*, plonk::*};
use std::marker::PhantomData;

#[derive(Default)]
pub(super) struct BoardCircuit<F: FieldExt> {
    ships: [[Value<F>; 3]; 5],
}

impl<F: FieldExt> Circuit<F> for BoardCircuit<F> {
    type Config = BoardConfig<F>;
    type FloorPlanner = floor_planner::V1;

    fn without_witnesses(&self) -> Self {
        Self::default()
    }

    fn configure(meta: &mut ConstraintSystem<F>) -> Self::Config {
        // configure public columns
        let ship_length = meta.fixed_column();
        // configure advice columns
        let x = meta.advice_column();
        let y = meta.advice_column();
        let z = meta.advice_column();

        // Toggle ship placement range constraint
        let q_range = meta.selector();

        let config = BoardConfig::<F> {
            ship_length,
            x,
            y,
            z,
            q_range,
            _marker: PhantomData,
        };
        BoardConfig::configure(meta, config)
    }

    fn synthesize(
        &self,
        config: Self::Config,
        mut layouter: impl Layouter<F>,
    ) -> Result<(), Error> {
        config.assign_ships(layouter.namespace(|| "Assign value"), self.ships)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{
        board_circuit::BoardCircuit,
        utils::{INVALID_SHIPS, VALID_SHIPS},
    };
    use halo2_proofs::{
        circuit::Value,
        dev::{FailureLocation, MockProver, VerifyFailure},
        pasta::Fp,
    };

    /**
     * Wrap ship tuples in Value::known
     */
    fn ship_to_values(ships: &[[u64; 3]; 5]) -> [[Value<Fp>; 3]; 5] {
        let empty = Value::known(Fp::zero());
        let empty_ship = [empty, empty, empty];
        let mut _ships: [[Value<Fp>; 3]; 5] =
            [empty_ship, empty_ship, empty_ship, empty_ship, empty_ship];
        for i in 0..ships.len() {
            for j in 0..ships[i].len() {
                _ships[i][j] = Value::known(Fp::from(ships[i][j]));
            }
        }
        _ships
    }

    #[test]
    fn test_board_circuit() {
        let k = 9;

        //------SHIP PLACEMENT RANGE CONSTRAINT VALIDATION------//
        // Successful cases
        for board in VALID_SHIPS {
            let circuit = BoardCircuit::<Fp> {
                ships: ship_to_values(&board),
            };
            let prover = MockProver::run(k, &circuit, vec![]).unwrap();
            prover.assert_satisfied();
        }
        // Unsuccessful cases

        // ship[1]: x range out of bounds (¬x∈[0, 9])
        let circuit = BoardCircuit::<Fp> {
            ships: ship_to_values(&INVALID_SHIPS[3]),
        };
        let prover = MockProver::run(k, &circuit, vec![]).unwrap();
        assert_eq!(
            prover.verify(),
            Err(vec![VerifyFailure::ConstraintNotSatisfied {
                constraint: ((0, "ship range check").into(), 0, "x decimal range check").into(),
                location: FailureLocation::InRegion {
                    region: (0, "Assign ships to advice cells").into(),
                    offset: 1
                },
                cell_values: vec![(((Any::Advice, 0).into(), 0).into(), "0xa".to_string())]
            }])
        );

        // ship[1]: y range out of bounds (¬x∈[0, 9])
        let circuit = BoardCircuit::<Fp> {
            ships: ship_to_values(&INVALID_SHIPS[4]),
        };
        let prover = MockProver::run(k, &circuit, vec![]).unwrap();
        assert_eq!(
            prover.verify(),
            Err(vec![VerifyFailure::ConstraintNotSatisfied {
                constraint: ((0, "ship range check").into(), 1, "y decimal range check").into(),
                location: FailureLocation::InRegion {
                    region: (0, "Assign ships to advice cells").into(),
                    offset: 1
                },
                cell_values: vec![(((Any::Advice, 1).into(), 0).into(), "0xb".to_string())]
            }])
        );

        // ship[1]: z range out of bounds (¬x∈[0, 1])
        let circuit = BoardCircuit::<Fp> {
            ships: ship_to_values(&INVALID_SHIPS[5]),
        };
        let prover = MockProver::run(k, &circuit, vec![]).unwrap();
        assert_eq!(
            prover.verify(),
            Err(vec![
                VerifyFailure::ConstraintNotSatisfied {
                    constraint: ((0, "ship range check").into(), 2, "z binary range check").into(),
                    location: FailureLocation::InRegion {
                        region: (0, "Assign ships to advice cells").into(),
                        offset: 1
                    },
                    cell_values: vec![(((Any::Advice, 2).into(), 0).into(), "0x2".to_string())]
                },
                VerifyFailure::ConstraintNotSatisfied {
                    // also fails the ship placement test
                    constraint: ((0, "ship range check").into(), 3, "ship length range check")
                        .into(),
                    location: FailureLocation::InRegion {
                        region: (0, "Assign ships to advice cells").into(),
                        offset: 1
                    },
                    cell_values: vec![
                        (((Any::Advice, 0).into(), 0).into(), "0x9".to_string()),
                        (((Any::Advice, 1).into(), 0).into(), "0x7".to_string()),
                        (((Any::Advice, 2).into(), 0).into(), "0x2".to_string()),
                        (((Any::Fixed, 0).into(), 0).into(), "0x4".to_string())
                    ]
                }
            ])
        );
        
        // ship[1] fails as z not toggled (ship is horizontal off board)
        let circuit = BoardCircuit::<Fp> {
            ships: ship_to_values(&INVALID_SHIPS[1]),
        };
        let prover = MockProver::run(k, &circuit, vec![]).unwrap();
        assert_eq!(
            prover.verify(),
            Err(vec![VerifyFailure::ConstraintNotSatisfied {
                constraint: ((0, "ship range check").into(), 3, "ship length range check").into(),
                location: FailureLocation::InRegion {
                    region: (0, "Assign ships to advice cells").into(),
                    offset: 1
                },
                cell_values: vec![
                    (((Any::Advice, 0).into(), 0).into(), "0x9".to_string()),
                    (((Any::Advice, 1).into(), 0).into(), "0x7".to_string()),
                    (((Any::Advice, 2).into(), 0).into(), "0".to_string()),
                    (((Any::Fixed, 0).into(), 0).into(), "0x4".to_string())
                ]
            }])
        );

        // ship 5 fails as z toggled (ship is vertical off board)
        let circuit = BoardCircuit::<Fp> {
            ships: ship_to_values(&INVALID_SHIPS[2]),
        };
        let prover = MockProver::run(k, &circuit, vec![]).unwrap();
        assert_eq!(
            prover.verify(),
            Err(vec![VerifyFailure::ConstraintNotSatisfied {
                constraint: ((0, "ship range check").into(), 3, "ship length range check").into(),
                location: FailureLocation::InRegion {
                    region: (0, "Assign ships to advice cells").into(),
                    offset: 4
                },
                cell_values: vec![
                    (((Any::Advice, 0).into(), 0).into(), "0".to_string()),
                    (((Any::Advice, 1).into(), 0).into(), "0".to_string()),
                    (((Any::Advice, 2).into(), 0).into(), "1".to_string()),
                    (((Any::Fixed, 0).into(), 0).into(), "0x2".to_string())
                ]
            }])
        );
    }
}
