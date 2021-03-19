use ckb_std::{
    ckb_constants::Source,
    high_level::{load_cell_data, load_script},
};
use core::convert::{TryFrom, TryInto};
use core::result::Result;
use das_core::{
    constants::{ScriptType, TypeScript},
    debug,
    error::Error,
    util,
};
use das_types::{constants::DataType, packed::*, prelude::*};

pub fn main() -> Result<(), Error> {
    debug!("====== Running apply-register-cell-type ======");

    let action_data = util::load_das_action()?;
    let action = action_data.as_reader().action().raw_data();
    if action == b"apply_register" {
        debug!("Route to apply_register action ...");

        let current = util::load_height()?;

        debug!("Reading ApplyRegisterCell ...");

        // Find out ApplyRegisterCells in current transaction.
        let this_type_script = load_script().map_err(|e| Error::from(e))?;
        let old_cells =
            util::find_cells_by_script(ScriptType::Type, &this_type_script, Source::Input)?;
        let new_cells =
            util::find_cells_by_script(ScriptType::Type, &this_type_script, Source::Output)?;

        // Consuming ApplyRegisterCell is not allowed in apply_register action.
        if old_cells.len() != 0 {
            return Err(Error::ApplyRegisterFoundInvalidTransaction);
        }
        // Only one ApplyRegisterCell can be created at one time.
        if new_cells.len() != 1 {
            return Err(Error::ApplyRegisterFoundInvalidTransaction);
        }

        // Verify the outputs_data of ApplyRegisterCell.
        let index = &new_cells[0];
        let data = load_cell_data(index.to_owned(), Source::Output).map_err(|e| Error::from(e))?;

        debug!("Check if first 32 bytes exists ...");

        // The first is a 32 bytes hash.
        match data.get(..32) {
            Some(bytes) => {
                Hash::try_from(bytes).map_err(|_| Error::InvalidCellData)?;
            }
            _ => return Err(Error::InvalidCellData),
        }

        debug!("Check if the ApplyRegisterCell and the HeightCell has the same height...");

        // Then follows the 8 bytes u64.
        let apply_height = match data.get(32..) {
            Some(bytes) => {
                if bytes.len() != 8 {
                    return Err(Error::InvalidCellData);
                }
                u64::from_le_bytes(bytes.try_into().unwrap())
            }
            _ => return Err(Error::InvalidCellData),
        };

        // The timestamp in ApplyRegisterCell must be the same as the timestamp in TimeCell.
        if apply_height != current {
            return Err(Error::ApplyRegisterCellHeightInvalid);
        }
    } else if action == b"pre_register" {
        debug!("Route to pre_register action ...");
        let mut parser = util::load_das_witnesses(Some(vec![DataType::ConfigCellMain]))?;
        util::require_type_script(
            &mut parser,
            TypeScript::PreAccountCellType,
            Source::Output,
            Error::PreRegisterFoundInvalidTransaction,
        )?;
    } else {
        return Err(Error::ActionNotSupported);
    }

    Ok(())
}
