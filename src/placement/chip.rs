use super::gadget::PlacementGadget;

use {
    crate::{
        bits2num::bits2num::{Bits2NumChip, Bits2NumConfig},
        placement::gadget::{InstructionUtilities, PlacementBits, PlacementState},
        utils::{
            binary::{bits_to_field_elements, unwrap_bitvec},
            ship::{PlacementUtilities, ShipPlacement},
            board::BOARD_SIZE
        },
    },
    halo2_proofs::{
        arithmetic::{lagrange_interpolate, FieldExt},
        circuit::{AssignedCell, Chip, Layouter, Region},
        plonk::{Advice, Column, ConstraintSystem, Constraints, Error, Expression, Selector},
        poly::Rotation,
    },
    std::marker::PhantomData,
};

/**
 * Storage required to use a ship placement validity chip
 * @TODO: BETTER DOCS HERE
 * @param bits2num - the Bits2NumConfig struct holding columns & data needed to compose bits into a decimal value
 * @param advice - array of 3 columns used to compute board validity
 *     * [bits, bit_sum, full_window_sum]
 *     * row 0 is padding for gate constraints
 *     * rows 1-100 perform running sum operations on bits. last row constrains output
 * @param selectors - array of 5 selectors that toggle constraints in chip
 *     * [placement_orientation, bit_sum, bit_adjacency, adjacency_permute, constrain_trace]
 * @param ship - Object storing/ exporting ship positioning
 */
#[derive(Clone, Copy, Debug)]
pub struct PlacementConfig<F: FieldExt, const S: usize> {
    pub bits2num: Bits2NumConfig,
    pub advice: [Column<Advice>; 3],
    pub selectors: [Selector; 5],
    _marker: PhantomData<F>,
}

pub struct PlacementChip<F: FieldExt, const S: usize> {
    config: PlacementConfig<F, S>,
}

impl<F: FieldExt, const S: usize> Chip<F> for PlacementChip<F, S> {
    type Config = PlacementConfig<F, S>;
    type Loaded = ();

    fn config(&self) -> &Self::Config {
        &self.config
    }

    fn loaded(&self) -> &Self::Loaded {
        &()
    }
}
pub trait PlacementInstructions<F: FieldExt, const S: usize> {
    /*
     * Loads decimal encoding of horizontal placement, vertical placement, and sum of the two
     * @dev constrains horizontal and vertical to be equal from other region
     *
     * @return - reference to assigned cells on which further constraints are performed
     */
    fn load_placement(
        &self,
        layouter: &mut impl Layouter<F>,
        horizontal: AssignedCell<F, F>,
        vertical: AssignedCell<F, F>,
    ) -> Result<[AssignedCell<F, F>; 3], Error>;

    /**
     * Generate a bits2num region and constrain it to equal a given assigned cell
     *
     * @param gadget - contains bit assigments
     * @param value - assigned call that bits2num should compose to (SUM(H, V))
     * @return - array of 100 assigned cells representing bits
     */
    fn synth_bits2num(
        &self,
        layouter: &mut impl Layouter<F>,
        gadget: PlacementGadget<F, S>,
        value: AssignedCell<F, F>,
    ) -> Result<PlacementBits<F>, Error>;

    /**
     * Generate the running sum for bit counts and full bit windows
     *
     * @param bits - 100 assigned bits to permute into this region
     * @param gadget - contains values for running sum trace to witness
     * @return - reference to final assignments for running bit sums and full bit window sums
     */
    fn placement_sums(
        &self,
        layouter: &mut impl Layouter<F>,
        bits: PlacementBits<F>,
        gadget: PlacementGadget<F, S>,
    ) -> Result<PlacementState<F>, Error>;

    /**
     * Constrain the witnessed running sum values for placement to be valid under game logic
     *
     * @param board_values - [horizontal, vertical] assignments
     * @param state - reference to assigned bit count and full bit window count cells
     */
    fn assign_constraint(
        &self,
        layouter: &mut impl Layouter<F>,
        state: PlacementState<F>,
    ) -> Result<(), Error>;
}

impl<F: FieldExt, const S: usize> PlacementChip<F, S> {
    pub fn new(config: PlacementConfig<F, S>) -> Self {
        PlacementChip { config }
    }

    /**
     * Configure the computation space of the circuit & return PlacementConfig
     */
    pub fn configure(meta: &mut ConstraintSystem<F>) -> PlacementConfig<F, S> {
        // allocate fixed column for constants
        let fixed = meta.fixed_column();
        meta.enable_equality(fixed);

        // define advice columns
        let mut advice = Vec::<Column<Advice>>::new();
        for _ in 0..3 {
            let col = meta.advice_column();
            meta.enable_equality(col);
            advice.push(col);
        }
        let advice: [Column<Advice>; 3] = advice.try_into().unwrap();

        // define selectors
        let mut selectors = Vec::<Selector>::new();
        for _ in 0..5 {
            selectors.push(meta.selector());
        }
        let selectors: [Selector; 5] = selectors.try_into().unwrap();

        // define bits2num config
        let bits2num = Bits2NumChip::<_, BOARD_SIZE>::configure(meta);

        // selector[0] gate: placement commitment constraint
        meta.create_gate("horizontal/ vertical placement constraint", |meta| {
            // retrieve witnessed cells
            let sum = meta.query_advice(advice[0], Rotation::cur());
            let horizontal = meta.query_advice(advice[1], Rotation::cur());
            let vertical = meta.query_advice(advice[2], Rotation::cur());
            // constain either horizontal or vertical placement to be 0
            let either_zero = horizontal.clone() * vertical.clone();
            // constrain sum == horizontal + vertical
            let summed = sum - (horizontal.clone() + vertical.clone());
            let selector = meta.query_selector(selectors[0]);
            Constraints::with_selector(
                selector,
                [("Either h or v == 0", either_zero), ("h + v = sum", summed)],
            )
        });

        // selector[1] gate: bit count running sum
        meta.create_gate("placement bit count", |meta| {
            // check that this row's bit count is sum of prev row's bit count + current row's bit value
            let bit = meta.query_advice(advice[0], Rotation::cur());
            // store running bit sum in advice[0]
            let prev = meta.query_advice(advice[1], Rotation::prev());
            let sum = meta.query_advice(advice[1], Rotation::cur());
            // constrain sum to be equal to bit + prev
            let selector = meta.query_selector(selectors[1]);
            Constraints::with_selector(selector, [("Running Sum: Bits", bit + prev - sum)])
        });

        // selector[2] gate: full bit window running sum
        meta.create_gate("adjacency bit count", |meta| {
            // count the number of bits in this gate and the proceeding `S` rows in bit column (A^2)
            let mut bit_count = meta.query_advice(advice[0], Rotation::cur());
            for i in 1..S {
                let bit = meta.query_advice(advice[0], Rotation(i as i32));
                bit_count = bit_count + bit;
            }

            // query full bit window running sum at column (A^4)
            let prev_full_window_count = meta.query_advice(advice[2], Rotation::prev());
            let full_window_count = meta.query_advice(advice[2], Rotation::cur());
            // constant expressions
            let ship_len = Expression::Constant(F::from(S as u64));
            let inverse_ship_len =
                Expression::Constant(F::from(S as u64).invert().unwrap_or(F::zero()));
            let one = Expression::Constant(F::one());

            /*
             * Raise a given expression to the given power
             *
             * @param base - the exponent base
             * @param pow - the power to raise the exponent base to
             * @return - the exponent base raised to power
             */
            let exp_pow = |base: Expression<F>, pow: usize| -> Expression<F> {
                let mut exp = base.clone();
                if pow == 0 {
                    exp = Expression::Constant(F::one())
                } else {
                    for i in 2..=pow {
                        exp = exp.clone() * base.clone();
                    }
                }
                exp
            };

            /*
             * Given a bit count, return the interpolated incrementor
             * @dev expects input to be in range [0, S]
             * @todo load lookup table with coefficients
             *
             * @param x - the sum of the bit window to pass in
             * @return - a boolean expression showing whether or not X = S (can be added as incrementor)
             */
            let interpolate_incrementor = |x: Expression<F>| -> Expression<F> {
                // generate lagrange interpolation inputs
                // if ship length is 4, then [0->0, 1->0, 2->0, 3->0, 4->1]
                let mut points = Vec::<F>::new();
                let mut evals = Vec::<F>::new();
                for i in 0..=S {
                    points.push(F::from(i as u64));
                    evals.push(if i == S { F::one() } else { F::zero() });
                }
                let interpolated = lagrange_interpolate(&points, &evals);
                let mut interpolated_value = Expression::Constant(F::zero());
                for i in 0..interpolated.len() {
                    let x_pow = exp_pow(x.clone(), i);
                    interpolated_value =
                        interpolated_value.clone() + Expression::Constant(interpolated[i]) * x_pow;
                }
                interpolated_value
            };

            // return constraint:
            // bit_count = bit_count
            // - if bit_count == ship_len, running_sum = prev_running_sum + 1
            // - if bit_count != ship_len, running_sum = prev_running
            let selector = meta.query_selector(selectors[2]);
            let constraint = full_window_count.clone()
                - prev_full_window_count
                - interpolate_incrementor(bit_count);
            Constraints::with_selector(selector, [("Full Window Running Sum", constraint)])
        });

        // selector[3] gate: permute bit window running sum
        meta.create_gate("permute adjaceny bit count", |meta| {
            // confirm that the current row's adjacent bit count is the same as the previous rows
            // @dev used in rows where ship cannot be placed (offset % 10 + ship_length >= 10)
            // store running adjacency count in advice[2]
            let previous = meta.query_advice(advice[2], Rotation::prev());
            let current = meta.query_advice(advice[2], Rotation::cur());
            // constrain previous to equal current
            let selector = meta.query_selector(selectors[3]);
            Constraints::with_selector(
                selector,
                [("Premute Full Window Running Sum", previous - current)],
            )
        });

        // selector[4] gate: constrain running sum trace
        meta.create_gate("running sum constraints", |meta| {
            // confirm the final output of the placement computation does not violate ship placement rules
            // @dev constraining of sum(h,v) to bits2num output occurs in synthesis
            let ship_len = Expression::Constant(F::from(S as u64));
            let one = Expression::Constant(F::one());
            let bit_count = meta.query_advice(advice[1], Rotation::cur());
            let full_window_count = meta.query_advice(advice[2], Rotation::cur());
            // - constrain bit count to be equal to S
            // - constrain exactly one full bit window
            let selector = meta.query_selector(selectors[4]);
            Constraints::with_selector(
                selector,
                [
                    ("Placed ship of correct length", bit_count - ship_len),
                    ("One full bit window", full_window_count - one),
                ],
            )
        });

        // export config
        PlacementConfig {
            bits2num,
            advice,
            selectors,
            _marker: PhantomData,
        }
    }

    pub fn synthesize(
        &self,
        mut layouter: impl Layouter<F>,
        horizontal: AssignedCell<F, F>,
        vertical: AssignedCell<F, F>,
        gadget: PlacementGadget<F, S>,
    ) -> Result<(), Error> {
        let placement_commitments = self.load_placement(&mut layouter, horizontal, vertical)?;
        let bits = self.synth_bits2num(&mut layouter, gadget, placement_commitments[0].clone())?;
        let running_sums = self.placement_sums(&mut layouter, bits, gadget)?;
        self.assign_constraint(&mut layouter, running_sums)?;
        Ok(())
    }
}

impl<F: FieldExt, const S: usize> PlacementInstructions<F, S> for PlacementChip<F, S> {
    fn load_placement(
        &self,
        layouter: &mut impl Layouter<F>,
        horizontal: AssignedCell<F, F>,
        vertical: AssignedCell<F, F>,
    ) -> Result<[AssignedCell<F, F>; 3], Error> {
        // variables used to construct witness

        let sum = horizontal.value().copied() + vertical.value().copied();
        // storage variable for assigned cell holding sum(h, v) to be constrained to bits2num
        let assigned: [AssignedCell<F, F>; 3] = layouter.assign_region(
            || "load placement encoded values",
            |mut region: Region<F>| {
                _ = self.config.selectors[0].enable(&mut region, 0);
                let sum = region.assign_advice(
                    || "sum of h & v placements",
                    self.config.advice[0],
                    0,
                    || sum,
                )?;
                let horizontal_cell = horizontal.copy_advice(
                    || "permute horizontal placement",
                    &mut region,
                    self.config.advice[1],
                    0,
                )?;
                let vertical_cell = vertical.copy_advice(
                    || "permute horizontal placement",
                    &mut region,
                    self.config.advice[2],
                    0,
                )?;
                Ok([sum, horizontal_cell.clone(), vertical_cell.clone()])
            },
        )?;
        Ok(assigned)
    }

    fn synth_bits2num(
        &self,
        layouter: &mut impl Layouter<F>,
        gadget: PlacementGadget<F, S>,
        value: AssignedCell<F, F>,
    ) -> Result<PlacementBits<F>, Error> {
        let bits2num = Bits2NumChip::<F, BOARD_SIZE>::new(value, gadget.bits);
        let assigned_bits =
            bits2num.synthesize(self.config.bits2num, layouter.namespace(|| "bits2num"))?;
        Ok(PlacementBits::<F>::from(assigned_bits))
    }

    fn placement_sums(
        &self,
        layouter: &mut impl Layouter<F>,
        bits2num: PlacementBits<F>,
        gadget: PlacementGadget<F, S>,
    ) -> Result<PlacementState<F>, Error> {
        Ok(layouter.assign_region(
            || "placement running sum trace",
            |mut region: Region<F>| {
                // pad first row with 0's to prevent running sums'
                // Rotation::prev() from unintended consequences
                let mut state = PlacementState::<F>::assign_padding_row(&mut region, &self.config)?;
                // permute bits constrained in "load placement encoded values" region to this region
                let _ = state.permute_bits2num(&bits2num, &mut region, &self.config)?;
                // // assign running sum trace across 100 (BOARD_SIZE) rows
                state = state.assign_running_sum_trace(&mut region, &self.config, &gadget)?;
                Ok(state)
            },
        )?)
    }

    fn assign_constraint(
        &self,
        layouter: &mut impl Layouter<F>,
        state: PlacementState<F>,
    ) -> Result<(), Error> {
        Ok(layouter.assign_region(
            || "constrain running sum output",
            |mut region: Region<F>| {
                state.bit_sum.copy_advice(
                    || "copy bit sum total count to constraint region",
                    &mut region,
                    self.config.advice[1],
                    0,
                )?;
                state.full_window_sum.copy_advice(
                    || "copy full bit window total count to constaint region",
                    &mut region,
                    self.config.advice[2],
                    0,
                )?;
                self.config.selectors[4].enable(&mut region, 0)?;
                Ok(())
            },
        )?)
    }
}
