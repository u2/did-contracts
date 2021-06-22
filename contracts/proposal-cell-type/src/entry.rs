use alloc::borrow::ToOwned;
use ckb_std::{
    ckb_constants::Source,
    ckb_types::bytes,
    high_level::{load_cell_capacity, load_cell_lock, load_cell_type, load_script},
};
use core::convert::TryFrom;
use core::result::Result;
use das_core::{
    assert,
    constants::*,
    data_parser::{account_cell, pre_account_cell},
    debug,
    error::Error,
    parse_witness, util,
    witness_parser::WitnessesParser,
};
use das_map::{map::Map, util as map_util};
use das_sorted_list::DasSortedList;
use das_types::{constants::*, packed::*, prelude::*};

pub fn main() -> Result<(), Error> {
    debug!("====== Running proposal-cell-type ======");

    let mut parser = WitnessesParser::new()?;
    util::is_system_off(&mut parser)?;

    debug!("Find out ProposalCell ...");

    // Find out PreAccountCells in current transaction.
    let this_type_script = load_script().map_err(|e| Error::from(e))?;
    let this_type_script_reader = this_type_script.as_reader();
    let (input_cells, output_cells) = util::find_cells_by_script_in_inputs_and_outputs(
        ScriptType::Type,
        this_type_script_reader,
    )?;
    let dep_cells =
        util::find_cells_by_script(ScriptType::Type, this_type_script_reader, Source::CellDep)?;

    let action_data = parser.parse_action()?;
    let action = action_data.as_reader().action().raw_data();
    if action == b"propose" {
        debug!("Route to propose action ...");

        parser.parse_cell()?;
        parser.parse_config(&[DataType::ConfigCellProposal])?;
        let config_main = parser.configs.main()?;
        let config_proposal = parser.configs.proposal()?;

        assert!(
            dep_cells.len() == 0 && input_cells.len() == 0 && output_cells.len() == 1,
            Error::ProposalFoundInvalidTransaction,
            "There should be only one ProposalCell found in the outputs."
        );

        util::is_cell_use_always_success_lock(output_cells[0], Source::Output)?;

        // Read outputs_data and witness of the ProposalCell.
        let index = &output_cells[0];
        let (_, _, entity) = parser.verify_and_get(index.to_owned(), Source::Output)?;
        let proposal_cell_data = ProposalCellData::from_slice(entity.as_reader().raw_data())
            .map_err(|_| Error::WitnessEntityDecodingError)?;
        let proposal_cell_data_reader = proposal_cell_data.as_reader();

        let required_cells_count =
            verify_slices(config_proposal, proposal_cell_data_reader.slices())?;
        let dep_related_cells = find_proposal_related_cells(config_main, Source::CellDep)?;

        #[cfg(not(feature = "mainnet"))]
        inspect_slices(proposal_cell_data_reader.slices())?;
        #[cfg(not(feature = "mainnet"))]
        inspect_related_cells(
            &parser,
            config_main,
            dep_related_cells.clone(),
            Source::CellDep,
            None,
        )?;

        assert!(
            required_cells_count == dep_related_cells.len(),
            Error::ProposalSliceRelatedCellMissing,
            "Some of the proposal relevant cells are missing. (expected: {}, current: {})",
            required_cells_count,
            dep_related_cells.len()
        );

        verify_slices_relevant_cells(
            config_main,
            proposal_cell_data_reader.slices(),
            dep_related_cells,
            None,
        )?;
    } else if action == b"extend_proposal" {
        debug!("Route to extend_proposal action ...");

        parser.parse_cell()?;
        parser.parse_config(&[DataType::ConfigCellProposal])?;
        let config_main = parser.configs.main()?;
        let config_proposal = parser.configs.proposal()?;

        assert!(
            dep_cells.len() == 1 && input_cells.len() == 0 && output_cells.len() == 1,
            Error::ProposalFoundInvalidTransaction,
            "There should be one ProposalCell found in the cell_deps and one in the outputs."
        );

        util::is_cell_use_always_success_lock(output_cells[0], Source::Output)?;

        // Read outputs_data and witness of previous ProposalCell.
        let index = &dep_cells[0];
        let (_, _, entity) = parser.verify_and_get(index.to_owned(), Source::CellDep)?;
        let prev_proposal_cell_data = ProposalCellData::from_slice(entity.as_reader().raw_data())
            .map_err(|_| Error::WitnessEntityDecodingError)?;
        let prev_proposal_cell_data_reader = prev_proposal_cell_data.as_reader();

        // Read outputs_data and witness of the ProposalCell.
        let index = &output_cells[0];
        let (_, _, entity) = parser.verify_and_get(index.to_owned(), Source::Output)?;
        let proposal_cell_data = ProposalCellData::from_slice(entity.as_reader().raw_data())
            .map_err(|_| Error::WitnessEntityDecodingError)?;
        let proposal_cell_data_reader = proposal_cell_data.as_reader();

        let required_cells_count =
            verify_slices(config_proposal, proposal_cell_data_reader.slices())?;
        let dep_related_cells = find_proposal_related_cells(config_main, Source::CellDep)?;

        #[cfg(not(feature = "mainnet"))]
        inspect_slices(proposal_cell_data_reader.slices())?;
        #[cfg(not(feature = "mainnet"))]
        inspect_related_cells(
            &parser,
            config_main,
            dep_related_cells.clone(),
            Source::CellDep,
            None,
        )?;

        assert!(
            required_cells_count == dep_related_cells.len(),
            Error::ProposalSliceRelatedCellMissing,
            "Some of the proposal relevant cells are missing. (expected: {}, current: {})",
            required_cells_count,
            dep_related_cells.len()
        );

        verify_slices_relevant_cells(
            config_main,
            proposal_cell_data_reader.slices(),
            dep_related_cells,
            Some(prev_proposal_cell_data_reader.slices()),
        )?;
    } else if action == b"confirm_proposal" {
        debug!("Route to confirm_proposal action ...");

        let timestamp = util::load_timestamp()?;
        // let height = util::load_height()?;

        parser.parse_cell()?;
        parser.parse_config(&[DataType::ConfigCellAccount, DataType::ConfigCellProfitRate])?;
        let config_account = parser.configs.account()?;
        let config_main = parser.configs.main()?;
        let config_profit_rate = parser.configs.profit_rate()?;

        assert!(
            dep_cells.len() == 0 && input_cells.len() == 1 && output_cells.len() == 0,
            Error::ProposalFoundInvalidTransaction,
            "There should be only one ProposalCell found in the inputs."
        );

        // Read outputs_data and witness of ProposalCell.
        let proposal_cell_index = input_cells[0];
        let proposal_cell_data =
            parse_witness!(parser, proposal_cell_index, Source::Input, ProposalCellData);
        let proposal_cell_data_reader = proposal_cell_data.as_reader();

        debug!("Check all AccountCells are updated or created base on proposal.");

        let input_related_cells = find_proposal_related_cells(config_main, Source::Input)?;
        let output_account_cells = find_output_account_cells(config_main)?;

        #[cfg(not(feature = "mainnet"))]
        inspect_slices(proposal_cell_data_reader.slices())?;
        #[cfg(not(feature = "mainnet"))]
        inspect_related_cells(
            &parser,
            config_main,
            input_related_cells.clone(),
            Source::Input,
            Some(output_account_cells.clone()),
        )?;

        verify_proposal_execution_result(
            &parser,
            config_account,
            config_main,
            config_profit_rate,
            timestamp,
            proposal_cell_data_reader,
            input_related_cells,
            output_account_cells,
        )?;

        verify_refund_correct(proposal_cell_index, proposal_cell_data_reader)?;
    } else if action == b"recycle_proposal" {
        debug!("Route to recycle_proposal action ...");

        parser.parse_cell()?;
        parser.parse_config(&[DataType::ConfigCellProposal])?;
        let config_proposal_reader = parser.configs.proposal()?;

        assert!(
            dep_cells.len() == 0 && input_cells.len() == 1 && output_cells.len() == 0,
            Error::ProposalFoundInvalidTransaction,
            "There should be only one ProposalCell found in the inputs."
        );

        debug!("Check if ProposalCell can be recycled.");

        let proposal_cell_index = input_cells[0];
        let (_, _, entity) = parser.verify_and_get(proposal_cell_index, Source::Input)?;
        let proposal_cell_data = ProposalCellData::from_slice(entity.as_reader().raw_data())
            .map_err(|_| Error::WitnessEntityDecodingError)?;
        let proposal_cell_data_reader = proposal_cell_data.as_reader();

        let height = util::load_height()?;
        let proposal_min_recycle_interval =
            u8::from(config_proposal_reader.proposal_min_recycle_interval()) as u64;
        let created_at_height = u64::from(proposal_cell_data_reader.created_at_height());

        assert!(
            height - created_at_height >= proposal_min_recycle_interval,
            Error::ProposalRecycleNeedWaitLonger,
            "ProposalCell should be recycled later, about {} block to wait.",
            created_at_height + proposal_min_recycle_interval - height
        );

        verify_refund_correct(proposal_cell_index, proposal_cell_data_reader)?;
    } else {
        return Err(Error::ActionNotSupported);
    }

    Ok(())
}

#[cfg(not(feature = "mainnet"))]
fn inspect_slices(slices_reader: SliceListReader) -> Result<(), Error> {
    debug!("Inspect Slices [");
    for (sl_index, sl_reader) in slices_reader.iter().enumerate() {
        debug!("  Slice[{}] [", sl_index);
        for (index, item) in sl_reader.iter().enumerate() {
            let type_ = item.item_type().raw_data()[0];
            let item_type = match type_ {
                0 => "exist",
                1 => "proposed",
                _ => "new",
            };

            debug!(
                "    Item[{}] {{ account_id: {:?}, item_type: {}, next: {:?} }}",
                index,
                item.account_id(),
                item_type,
                item.next()
            );
        }
        debug!("  ]");
    }
    debug!("]");

    Ok(())
}

#[cfg(not(feature = "mainnet"))]
fn inspect_related_cells(
    parser: &WitnessesParser,
    config_main: ConfigCellMainReader,
    related_cells: Vec<usize>,
    related_cells_source: Source,
    output_account_cells: Option<Vec<usize>>,
) -> Result<(), Error> {
    use das_core::inspect;

    debug!("Inspect {:?}:", related_cells_source);
    for i in related_cells {
        let script = load_cell_type(i, related_cells_source)
            .map_err(|e| Error::from(e))?
            .unwrap();
        let code_hash = Hash::from(script.code_hash());
        let (_, _, entity) = parser.verify_and_get(i, related_cells_source)?;
        let data = util::load_cell_data(i, related_cells_source)?;

        debug!("  Input[{}].cell.type: {}", i, script);

        if util::is_reader_eq(
            config_main.type_id_table().account_cell(),
            code_hash.as_reader(),
        ) {
            inspect::account_cell(Source::Input, i, &data, entity.to_owned());
        } else if util::is_reader_eq(
            config_main.type_id_table().pre_account_cell(),
            code_hash.as_reader(),
        ) {
            inspect::pre_account_cell(Source::Input, i, &data, entity.to_owned());
        }
    }

    if let Some(output_account_cells) = output_account_cells {
        for i in output_account_cells {
            let script = load_cell_type(i, Source::Output)
                .map_err(|e| Error::from(e))?
                .unwrap();
            let code_hash = Hash::from(script.code_hash());
            let (_, _, entity) = parser.verify_and_get(i, Source::Output)?;
            let data = util::load_cell_data(i, Source::Output)?;

            debug!("  Output[{}].cell.type: {}", i, script);

            if util::is_reader_eq(
                config_main.type_id_table().account_cell(),
                code_hash.as_reader(),
            ) {
                inspect::account_cell(Source::Output, i, &data, entity.to_owned());
            }
        }
    }

    Ok(())
}

fn verify_slices(
    config: ConfigCellProposalReader,
    slices_reader: SliceListReader,
) -> Result<usize, Error> {
    debug!("Check the data structure of proposal slices.");

    // debug!("slices_reader = {}", slices_reader);

    let mut required_cells_count: usize = 0;
    let mut exists_account_ids: Vec<bytes::Bytes> = Vec::new();
    let mut account_cell_contained = 0;
    let mut pre_account_cell_contained = 0;

    assert!(
        slices_reader.len() > 0,
        Error::ProposalSlicesCanNotBeEmpty,
        "The slices of ProposalCell should not be empty."
    );

    for (sl_index, sl_reader) in slices_reader.iter().enumerate() {
        debug!("Check Slice[{}] ...", sl_index);
        let mut account_id_list = Vec::new();

        assert!(
            sl_reader.len() > 1,
            Error::ProposalSliceMustContainMoreThanOneElement,
            "Slice[{}] must contain more than one element, but {} found.",
            sl_index,
            sl_reader.len()
        );

        // The "next" of last item is refer to an existing account, so we put it into the vector.
        let first_item = sl_reader.get(0).unwrap();
        exists_account_ids.push(bytes::Bytes::from(first_item.account_id().raw_data()));
        let last_item = sl_reader.get(sl_reader.len() - 1).unwrap();
        exists_account_ids.push(bytes::Bytes::from(last_item.next().raw_data()));

        for (index, item) in sl_reader.iter().enumerate() {
            debug!("  Check if Item[{}] refer to correct next.", index);

            if index == 0 {
                account_cell_contained += 1;
                assert!(
                    u8::from(item.item_type()) != ProposalSliceItemType::New as u8,
                    Error::ProposalCellTypeError,
                    "  Item[{}] The item_type of item[{}] should not be {:?}.",
                    index,
                    index,
                    ProposalSliceItemType::New
                )
            } else {
                pre_account_cell_contained += 1;
                assert!(
                    u8::from(item.item_type()) == ProposalSliceItemType::New as u8,
                    Error::ProposalCellTypeError,
                    "  Item[{}] The item_type of item[{}] should be {:?}.",
                    index,
                    index,
                    ProposalSliceItemType::New
                )
            }

            // Check the uniqueness of current account.
            let account_id_bytes = bytes::Bytes::from(item.account_id().raw_data());
            if index != 0 {
                for account_id in exists_account_ids.iter() {
                    assert!(
                        account_id.ne(account_id_bytes.as_ref()),
                        Error::ProposalSliceItemMustBeUniqueAccount,
                        "  Item[{}] is an exists account.",
                        index
                    );
                }
            }

            // Check the continuity of the items in the slice.
            if let Some(next_item) = sl_reader.get(index + 1) {
                assert!(
                    util::is_reader_eq(item.next(), next_item.account_id()),
                    Error::ProposalSliceIsDiscontinuity,
                    "  Item[{}].next should be {}, but it is {} now.",
                    index,
                    next_item.account_id(),
                    item.next()
                );
            }

            // Store exists account IDs for uniqueness verification.
            exists_account_ids.push(account_id_bytes.clone());
            // Store account IDs for order verification.
            account_id_list.push(account_id_bytes);
            required_cells_count += 1;
        }

        // Check the order of items in the slice.
        let sorted_account_id_list = DasSortedList::new(account_id_list.clone());
        assert!(
            sorted_account_id_list.cmp_order_with(account_id_list),
            Error::ProposalSliceIsNotSorted,
            "The order of items in Slice[{}] is incorrect.",
            sl_index
        );
    }

    let max_account_cell_count = u32::from(config.proposal_max_account_affect());
    assert!(
        account_cell_contained < max_account_cell_count,
        Error::ProposalFoundInvalidTransaction,
        "The proposal should not contains more than {} AccountCells.",
        max_account_cell_count
    );

    let max_pre_account_cell_count = u32::from(config.proposal_max_pre_account_contain());
    assert!(
        pre_account_cell_contained < max_pre_account_cell_count,
        Error::ProposalFoundInvalidTransaction,
        "The proposal should not contains more than {} PreAccountCells.",
        max_pre_account_cell_count
    );

    Ok(required_cells_count)
}

fn find_proposal_related_cells(
    config: ConfigCellMainReader,
    source: Source,
) -> Result<Vec<usize>, Error> {
    // Find related cells' indexes in cell_deps or inputs.
    let account_cell_type_id = config.type_id_table().account_cell();
    let account_cells =
        util::find_cells_by_type_id(ScriptType::Type, account_cell_type_id, source)?;
    let pre_account_cell_type_id = config.type_id_table().pre_account_cell();
    let pre_account_cells =
        util::find_cells_by_type_id(ScriptType::Type, pre_account_cell_type_id, source)?;

    assert!(
        pre_account_cells.len() > 0,
        Error::ProposalFoundInvalidTransaction,
        "There should be some PreAccountCells in {:?}.",
        source
    );

    // Merge cells' indexes in sorted order.
    let mut sorted = Vec::new();
    if account_cells.len() > 0 {
        let mut i = 0;
        let mut j = 0;
        let remain;
        let remain_idx;
        loop {
            if account_cells[i] < pre_account_cells[j] {
                sorted.push(account_cells[i]);
                i += 1;
                if i == account_cells.len() {
                    remain = pre_account_cells;
                    remain_idx = j;
                    break;
                }
            } else {
                sorted.push(pre_account_cells[j]);
                j += 1;
                if j == pre_account_cells.len() {
                    remain = account_cells;
                    remain_idx = i;
                    break;
                }
            }
        }

        for i in remain_idx..remain.len() {
            sorted.push(remain[i]);
        }
    } else {
        // The PreAccountCells in inputs is already sorted by their indexes, so no need to sort again.
        sorted = pre_account_cells;
    }

    debug!(
        "Inputs cells(AccountCell/PreAccountCell) sorted index list: {:?}",
        sorted
    );

    Ok(sorted)
}

fn find_output_account_cells(config: ConfigCellMainReader) -> Result<Vec<usize>, Error> {
    // Find updated cells' indexes in outputs.
    let account_cell_type_id = config.type_id_table().account_cell();
    let mut account_cells =
        util::find_cells_by_type_id(ScriptType::Type, account_cell_type_id, Source::Output)?;
    account_cells.sort();

    assert!(
        account_cells.len() > 0,
        Error::ProposalFoundInvalidTransaction,
        "There should be some AccountCells in the outputs."
    );

    debug!(
        "Outputs cells(AccountCell) sorted index list: {:?}",
        account_cells
    );

    Ok(account_cells)
}

fn verify_slices_relevant_cells(
    config: ConfigCellMainReader,
    slices_reader: SliceListReader,
    relevant_cells: Vec<usize>,
    prev_slices_reader_opt: Option<SliceListReader>,
) -> Result<(), Error> {
    debug!("Check the proposal slices relevant cells are real exist and in correct status.");

    let mut i = 0;
    for (sl_index, sl_reader) in slices_reader.iter().enumerate() {
        debug!("Check slice {} ...", sl_index);
        let mut next_of_first_cell = AccountId::default();
        for (item_index, item) in sl_reader.iter().enumerate() {
            let item_account_id = item.account_id();
            let item_type = u8::from(item.item_type());

            let cell_index = relevant_cells[i];

            // Check if the relevant cells has the same type as in the proposal.
            let expected_type_id = if item_type == ProposalSliceItemType::Exist as u8 {
                config.type_id_table().account_cell()
            } else {
                config.type_id_table().pre_account_cell()
            };
            verify_cell_type_id(item_index, cell_index, Source::CellDep, &expected_type_id)?;

            let cell_data = util::load_cell_data(cell_index, Source::CellDep)?;
            // Check if the relevant cells have the same account ID as in the proposal.
            verify_account_cell_account_id(
                item_index,
                &cell_data,
                cell_index,
                Source::CellDep,
                item_account_id.raw_data(),
            )?;

            // ⚠️ The first item is very very important, its "next" must be correct so that
            // AccountCells can form a linked list.
            if item_index == 0 {
                // If this is the first proposal in proposal chain, all slice must start with an AccountCell.
                if prev_slices_reader_opt.is_none() {
                    assert!(
                        item_type == ProposalSliceItemType::Exist as u8,
                        Error::ProposalSliceMustStartWithAccountCell,
                        "  In the first proposal of a proposal chain, all slice should start with an AccountCell."
                    );

                    // The correct "next" of first proposal is come from the cell's outputs_data.
                    next_of_first_cell = AccountId::try_from(account_cell::get_next(&cell_data))
                        .map_err(|_| Error::InvalidCellData)?;

                // If this is the extended proposal in proposal chain, slice may starting with an
                // AccountCell/PreAccountCell included in previous proposal, or it may starting with
                // an AccountCell not included in previous proposal.
                } else {
                    assert!(
                        item_type == ProposalSliceItemType::Exist as u8 || item_type == ProposalSliceItemType::Proposed as u8,
                        Error::ProposalSliceMustStartWithAccountCell,
                        "  In the extended proposal of a proposal chain, slices should start with an AccountCell or a PreAccountCell which included in previous proposal."
                    );

                    let prev_slices_reader = prev_slices_reader_opt.as_ref().unwrap();
                    next_of_first_cell =
                        match find_item_contains_account_id(prev_slices_reader, &item_account_id) {
                            // If the item is included in previous proposal, then we need to get its latest "next" from the proposal.
                            Ok(prev_item) => prev_item.next(),
                            // If the item is not included in previous proposal, then we get its latest "next" from the cell's outputs_data.
                            Err(_) => AccountId::try_from(account_cell::get_next(&cell_data))
                                .map_err(|_| Error::InvalidCellData)?,
                        };
                }
            }

            i += 1;
        }

        // Check if the first item's "next" has pass to the last item correctly.
        let item = sl_reader.get(sl_reader.len() - 1).unwrap();
        let next_of_last_item = item.next();

        assert!(
            util::is_reader_eq(next_of_first_cell.as_reader(), next_of_last_item),
            Error::ProposalSliceNotEndCorrectly,
            "The next of first item should be pass to the last item correctly."
        );
    }

    Ok(())
}

fn find_item_contains_account_id(
    prev_slices_reader: &SliceListReader,
    account_id: &AccountIdReader,
) -> Result<ProposalItem, Error> {
    for slice in prev_slices_reader.iter() {
        for item in slice.iter() {
            if util::is_reader_eq(item.account_id(), *account_id) {
                return Ok(item.to_entity());
            }
        }
    }

    debug!("Can not find previous item: {}", account_id);
    Err(Error::PrevProposalItemNotFound)
}

fn verify_proposal_execution_result(
    parser: &WitnessesParser,
    config_account: ConfigCellAccountReader,
    config_main: ConfigCellMainReader,
    config_profit_rate: ConfigCellProfitRateReader,
    timestamp: u64,
    proposal_cell_data_reader: ProposalCellDataReader,
    input_related_cells: Vec<usize>,
    output_account_cells: Vec<usize>,
) -> Result<(), Error> {
    debug!("Check that all AccountCells/PreAccountCells have been converted according to the proposal.");

    let das_wallet_lock = das_wallet_lock();
    let proposer_lock_reader = proposal_cell_data_reader.proposer_lock();
    let slices_reader = proposal_cell_data_reader.slices();
    let account_cell_type_id = config_main.type_id_table().account_cell();
    let pre_account_cell_type_id = config_main.type_id_table().pre_account_cell();

    let mut profit_map = Map::new();
    let inviter_profit_rate = u32::from(config_profit_rate.inviter()) as u64;
    let channel_profit_rate = u32::from(config_profit_rate.channel()) as u64;
    let proposal_create_profit_rate = u32::from(config_profit_rate.proposal_create()) as u64;
    let proposal_confirm_profit_rate = u32::from(config_profit_rate.proposal_confirm()) as u64;

    let mut i = 0;
    for (sl_index, sl_reader) in slices_reader.iter().enumerate() {
        debug!("Check Slice[{}] ...", sl_index);
        for (item_index, item) in sl_reader.iter().enumerate() {
            let item_account_id = item.account_id().raw_data();
            let item_type = u8::from(item.item_type());
            let item_next = item.next();

            let input_cell_data = util::load_cell_data(input_related_cells[i], Source::Input)?;
            let output_cell_data = util::load_cell_data(output_account_cells[i], Source::Output)?;

            if item_type == ProposalSliceItemType::Exist as u8
                || item_type == ProposalSliceItemType::Proposed as u8
            {
                debug!(
                    "  Item[{}] Check that the existing inputs[{}].AccountCell and outputs[{}].AccountCell is updated correctly.",
                    item_index, input_related_cells[i], output_account_cells[i]
                );

                // All cells' type is must be account-cell-type
                verify_cell_type_id(
                    item_index,
                    input_related_cells[i],
                    Source::Input,
                    &account_cell_type_id,
                )?;
                verify_cell_type_id(
                    item_index,
                    output_account_cells[i],
                    Source::Output,
                    &account_cell_type_id,
                )?;

                // All cells' account_id in data must be the same as the account_id in proposal.
                verify_account_cell_account_id(
                    item_index,
                    &input_cell_data,
                    input_related_cells[i],
                    Source::Input,
                    item_account_id,
                )?;
                verify_account_cell_account_id(
                    item_index,
                    &output_cell_data,
                    output_account_cells[i],
                    Source::Output,
                    item_account_id,
                )?;

                util::is_cell_capacity_equal(
                    (input_related_cells[i], Source::Input),
                    (output_account_cells[i], Source::Output),
                )?;
                util::is_cell_lock_equal(
                    (input_related_cells[i], Source::Input),
                    (output_account_cells[i], Source::Output),
                )?;

                // For the existing AccountCell, only the next field in data can be modified.
                // No need to check the witness of AccountCells here, because we check their hash instead.
                is_old_account_cell_data_consistent(
                    item_index,
                    &output_cell_data,
                    &input_cell_data,
                )?;
                is_next_correct(item_index, &output_cell_data, item_next)?;
            } else {
                debug!(
                    "  Item[{}] Check that the inputs[{}].PreAccountCell and outputs[{}].AccountCell is converted correctly.",
                    item_index, input_related_cells[i], output_account_cells[i]
                );

                // All cells' type is must be pre-account-cell-type/account-cell-type
                verify_cell_type_id(
                    item_index,
                    input_related_cells[i],
                    Source::Input,
                    &pre_account_cell_type_id,
                )?;
                verify_cell_type_id(
                    item_index,
                    output_account_cells[i],
                    Source::Output,
                    &account_cell_type_id,
                )?;

                // All cells' account_id in data must be the same as the account_id in proposal.
                verify_pre_account_cell_account_id(
                    item_index,
                    &input_cell_data,
                    input_related_cells[i],
                    Source::Input,
                    item_account_id,
                )?;
                verify_account_cell_account_id(
                    item_index,
                    &output_cell_data,
                    output_account_cells[i],
                    Source::Output,
                    item_account_id,
                )?;

                let output_cell_witness = parse_witness!(
                    parser,
                    input_related_cells[i],
                    Source::Input,
                    PreAccountCellData
                );
                let input_cell_witness_reader = output_cell_witness.as_reader();

                let output_cell_witness = parse_witness!(
                    parser,
                    output_account_cells[i],
                    Source::Output,
                    AccountCellData
                );
                let output_cell_witness_reader = output_cell_witness.as_reader();

                let account_name_storage =
                    account_cell::get_account(&output_cell_data).len() as u64;
                let total_capacity = load_cell_capacity(input_related_cells[i], Source::Input)
                    .map_err(|e| Error::from(e))?;
                let storage_capacity =
                    util::calc_account_storage_capacity(config_account, account_name_storage);
                // Allocate the profits carried by PreAccountCell to the wallets for later verification.
                let profit = total_capacity - storage_capacity;

                debug!(
                    "  Item[{}] The profit in PreAccountCell is: {}(profit) = {}(total_capacity) - {}(storage_capacity)",
                    item_index, profit, total_capacity, storage_capacity
                );

                util::verify_account_length_and_years(
                    input_cell_witness_reader.account().len(),
                    timestamp,
                    Some(item_index),
                )?;

                is_cell_capacity_correct(item_index, output_account_cells[i], storage_capacity)?;
                is_new_account_cell_lock_correct(
                    item_index,
                    input_related_cells[i],
                    input_cell_witness_reader,
                    output_account_cells[i],
                )?;

                // Check all fields in the data of new AccountCell.
                is_id_correct(item_index, &output_cell_data, &input_cell_data)?;
                is_account_correct(item_index, &output_cell_data)?;
                is_next_correct(item_index, &output_cell_data, item_next)?;
                is_expired_at_correct(
                    item_index,
                    profit,
                    timestamp,
                    &output_cell_data,
                    input_cell_witness_reader,
                )?;

                // Check all fields in the witness of new AccountCell.
                verify_witness_id(item_index, &output_cell_data, output_cell_witness_reader)?;
                verify_witness_account(item_index, &output_cell_data, output_cell_witness_reader)?;
                verify_witness_status(item_index, output_cell_witness_reader)?;

                let mut inviter_profit = 0;
                if input_cell_witness_reader.inviter_lock().is_some() {
                    let inviter_lock_reader =
                        input_cell_witness_reader.inviter_lock().to_opt().unwrap();
                    inviter_profit = profit * inviter_profit_rate / RATE_BASE;
                    debug!(
                        "  Item[{}] lock.args[{}]: {}(inviter_profit) = {}(profit) * {}(inviter_profit_rate) / {}(RATE_BASE)",
                        item_index, inviter_lock_reader.args(), inviter_profit, profit, inviter_profit_rate, RATE_BASE
                    );
                    map_util::add(
                        &mut profit_map,
                        inviter_lock_reader.as_slice().to_vec(),
                        inviter_profit,
                    );
                };

                let mut channel_profit = 0;
                if input_cell_witness_reader.channel_lock().is_some() {
                    let channel_lock_reader =
                        input_cell_witness_reader.channel_lock().to_opt().unwrap();
                    channel_profit = profit * channel_profit_rate / RATE_BASE;
                    debug!(
                        "  Item[{}] lock.args[{}]: {}(channel_profit) = {}(profit) * {}(channel_profit_rate) / {}(RATE_BASE)",
                        item_index, channel_lock_reader.args(), channel_profit, profit, channel_profit_rate, RATE_BASE
                    );
                    map_util::add(
                        &mut profit_map,
                        channel_lock_reader.as_slice().to_vec(),
                        channel_profit,
                    );
                };

                let proposal_create_profit = profit * proposal_create_profit_rate / RATE_BASE;
                debug!(
                    "  Item[{}] lock.args[{}]: {}(proposal_create_profit) = {}(profit) * {}(proposal_create_profit_rate) / {}(RATE_BASE)",
                    item_index, proposer_lock_reader.args(), proposal_create_profit, profit, proposal_create_profit_rate, RATE_BASE
                );
                map_util::add(
                    &mut profit_map,
                    proposer_lock_reader.as_slice().to_vec(),
                    proposal_create_profit,
                );

                let proposal_confirm_profit = profit * proposal_confirm_profit_rate / RATE_BASE;
                debug!(
                    "  Item[{}] {}(proposal_confirm_profit) = {}(profit) * {}(proposal_confirm_profit_rate) / {}(RATE_BASE) (! not included in IncomeCell)",
                    item_index, proposal_confirm_profit, profit, proposal_confirm_profit_rate, RATE_BASE
                );
                // No need to record proposal confirm profit, bacause the transaction creator can take its profit freely and this script do not know which lock script the transaction creator will use.

                let das_profit = profit
                    - inviter_profit
                    - channel_profit
                    - proposal_create_profit
                    - proposal_confirm_profit;
                map_util::add(
                    &mut profit_map,
                    das_wallet_lock.as_reader().as_slice().to_vec(),
                    das_profit,
                );

                debug!(
                    "  Item[{}] lock.args[{}]: {}(das_profit) = {}(profit) - {}(inviter_profit) - {}(channel_profit) - {}(proposal_create_profit) - {}(proposal_confirm_profit)",
                    item_index, das_wallet_lock.as_reader().args(), das_profit, profit, inviter_profit, channel_profit, proposal_create_profit, proposal_confirm_profit
                );
            }

            i += 1;
        }
    }

    debug!("Check if the IncomeCell in inputs is a newly created IncomeCell with only one record.");

    let income_cell_type_id = config_main.type_id_table().income_cell();
    let input_income_cells =
        util::find_cells_by_type_id(ScriptType::Type, income_cell_type_id, Source::Input)?;
    let output_income_cells =
        util::find_cells_by_type_id(ScriptType::Type, income_cell_type_id, Source::Output)?;

    assert!(
        input_income_cells.len() <= 1,
        Error::ProposalFoundInvalidTransaction,
        "The number of IncomeCells in inputs should be less than or equal to 1. (expected: <= 1, current: {})",
        input_income_cells.len()
    );

    if input_income_cells.len() == 1 {
        let (_, _, entity) = parser.verify_and_get(input_income_cells[0], Source::Input)?;
        let income_cell_witness = IncomeCellData::from_slice(entity.as_reader().raw_data())
            .map_err(|_| Error::WitnessEntityDecodingError)?;
        let income_cell_witness_reader = income_cell_witness.as_reader();

        // The IncomeCell should be a newly created cell with only one record which is belong to the creator, but we do not need to check everything here, so we only check the length.
        assert!(
            income_cell_witness_reader.records().len() == 1,
            Error::ProposalFoundInvalidTransaction,
            "The IncomeCell in inputs should be a newly created cell with only one record which is belong to the creator."
        );

        // Add the original record into profit_map to bypass later verification.
        let first_record = income_cell_witness_reader.records().get(0).unwrap();
        profit_map.insert(
            first_record.belong_to().as_slice().to_vec(),
            u64::from(first_record.capacity()),
        );
    }

    debug!("Check if the IncomeCell in outputs records everyone's profit correctly.");

    assert!(
        output_income_cells.len() == 1,
        Error::ProposalFoundInvalidTransaction,
        "The number of IncomeCells in outputs should be exactly 1 . (expected: == 1, current: {})",
        output_income_cells.len()
    );

    let (_, _, entity) = parser.verify_and_get(output_income_cells[0], Source::Output)?;
    let output_cell_witness = IncomeCellData::from_slice(entity.as_reader().raw_data())
        .map_err(|_| Error::WitnessEntityDecodingError)?;
    let output_cell_witness_reader = output_cell_witness.as_reader();
    let mut expected_capacity = 0;

    for (i, record) in output_cell_witness_reader.records().iter().enumerate() {
        let key = record.belong_to().as_slice().to_vec();
        let recorded_profit = u64::from(record.capacity());
        let result = profit_map.get(&key);

        assert!(
            result.is_some(),
            Error::ProposalConfirmIncomeError,
            "  IncomeCell.records[{}] Found a profit record which should not be in the IncomeCell.records, please compare the locks in PreAccountCells and ProposalCells with the belong_to field. (belong_to: {})",
            i,
            record.belong_to()
        );

        let expected_profit = result.unwrap();
        assert!(
            &recorded_profit == expected_profit,
            Error::ProposalConfirmIncomeError,
            "  IncomeCell.records[{}] The capacity of a profit record is incorrect. (expected: {}, current: {}, belong_to: {})",
            i,
            expected_profit,
            recorded_profit,
            record.belong_to()
        );

        profit_map.remove(&key);
        expected_capacity += recorded_profit;
    }

    assert!(
        profit_map.is_empty(),
        Error::ProposalConfirmIncomeError,
        "The IncomeCell in outputs should contains everyone's profit. (missing: {})",
        profit_map.len()
    );

    let current_capacity =
        load_cell_capacity(output_income_cells[0], Source::Output).map_err(|e| Error::from(e))?;
    assert!(
        expected_capacity == current_capacity,
        Error::ProposalConfirmIncomeError,
        "The capacity of the IncomeCell shoulde be {}, but {} found.",
        expected_capacity,
        current_capacity
    );

    Ok(())
}

fn verify_cell_type_id(
    item_index: usize,
    cell_index: usize,
    source: Source,
    expected_type_id: &HashReader,
) -> Result<(), Error> {
    let cell_type_id = load_cell_type(cell_index, source)
        .map_err(|e| Error::from(e))?
        .map(|script| script.code_hash())
        .ok_or(Error::ProposalSliceRelatedCellNotFound)?;

    assert!(
        cell_type_id.as_reader().raw_data() == expected_type_id.raw_data(),
        Error::ProposalCellTypeError,
        "  The type ID of Item[{}] should be {}. (related_cell: {:?}[{}])",
        item_index,
        expected_type_id,
        source,
        cell_index
    );

    Ok(())
}

fn verify_account_cell_account_id(
    item_index: usize,
    cell_data: &Vec<u8>,
    cell_index: usize,
    source: Source,
    expected_account_id: &[u8],
) -> Result<(), Error> {
    let account_id = account_cell::get_id(&cell_data);

    assert!(
        account_id == expected_account_id,
        Error::ProposalCellAccountIdError,
        "  The account ID of Item[{}] should be {}. (related_cell: {:?}[{}])",
        item_index,
        util::hex_string(expected_account_id),
        source,
        cell_index
    );

    Ok(())
}

fn verify_pre_account_cell_account_id(
    item_index: usize,
    cell_data: &Vec<u8>,
    cell_index: usize,
    source: Source,
    expected_account_id: &[u8],
) -> Result<(), Error> {
    let account_id = pre_account_cell::get_id(&cell_data);

    assert!(
        account_id == expected_account_id,
        Error::ProposalCellAccountIdError,
        "  The account ID of Item[{}] should be {}. (related_cell: {:?}[{}])",
        item_index,
        util::hex_string(expected_account_id),
        source,
        cell_index
    );

    Ok(())
}

fn is_new_account_cell_lock_correct(
    item_index: usize,
    input_cell_index: usize,
    input_cell_witness_reader: PreAccountCellDataReader,
    output_cell_index: usize,
) -> Result<(), Error> {
    debug!(
        "  Item[{}] Check if the lock script of new AccountCells is das-lock.",
        item_index
    );

    let das_lock = das_lock();
    let owner_lock_args = input_cell_witness_reader
        .owner_lock_args()
        .raw_data()
        .to_owned();
    let output_cell_lock =
        load_cell_lock(output_cell_index, Source::Output).map_err(|e| Error::from(e))?;

    let expected_lock = das_lock
        .as_builder()
        .args(Bytes::from(owner_lock_args).into())
        .build();

    assert!(
        util::is_entity_eq(&expected_lock, &output_cell_lock),
        Error::ProposalConfirmAccountLockArgsIsInvalid,
        "  Item[{}] The outputs[{}].lock should come from the owner_lock_args of inputs[{}]. (expected: {}, current: {})",
        item_index,
        output_cell_index,
        input_cell_index,
        expected_lock,
        output_cell_lock
    );

    Ok(())
}

fn is_bytes_eq(
    item_index: usize,
    field: &str,
    current_bytes: &[u8],
    expected_bytes: &[u8],
    error_code: Error,
) -> Result<(), Error> {
    assert!(
        current_bytes == expected_bytes,
        error_code,
        "  Item[{}] The AccountCell.{} should be consist in inputs and outputs.(expected: {}, current: {})",
        item_index,
        field,
        util::hex_string(expected_bytes),
        util::hex_string(current_bytes)
    );

    Ok(())
}

fn is_old_account_cell_data_consistent(
    item_index: usize,
    output_cell_data: &Vec<u8>,
    input_cell_data: &Vec<u8>,
) -> Result<(), Error> {
    is_bytes_eq(
        item_index,
        "hash",
        output_cell_data.get(..32).unwrap(),
        input_cell_data.get(..32).unwrap(),
        Error::ProposalFieldCanNotBeModified,
    )?;
    is_bytes_eq(
        item_index,
        "id",
        account_cell::get_id(output_cell_data),
        account_cell::get_id(input_cell_data),
        Error::ProposalFieldCanNotBeModified,
    )?;
    is_bytes_eq(
        item_index,
        "account",
        account_cell::get_account(output_cell_data),
        account_cell::get_account(input_cell_data),
        Error::ProposalFieldCanNotBeModified,
    )?;
    is_bytes_eq(
        item_index,
        "expired_at",
        &account_cell::get_expired_at(output_cell_data).to_le_bytes(),
        &account_cell::get_expired_at(input_cell_data).to_le_bytes(),
        Error::ProposalFieldCanNotBeModified,
    )?;

    Ok(())
}

fn is_id_correct(
    item_index: usize,
    output_cell_data: &Vec<u8>,
    input_cell_data: &Vec<u8>,
) -> Result<(), Error> {
    is_bytes_eq(
        item_index,
        "id",
        account_cell::get_id(output_cell_data),
        account_cell::get_id(input_cell_data),
        Error::ProposalConfirmNewAccountCellDataError,
    )
}

fn is_next_correct(
    item_index: usize,
    output_cell_data: &Vec<u8>,
    proposed_next: AccountIdReader,
) -> Result<(), Error> {
    let expected_next = proposed_next.raw_data();

    is_bytes_eq(
        item_index,
        "next",
        account_cell::get_next(output_cell_data),
        expected_next,
        Error::ProposalConfirmNewAccountCellDataError,
    )
}

fn is_expired_at_correct(
    item_index: usize,
    profit: u64,
    current_timestamp: u64,
    output_cell_data: &Vec<u8>,
    pre_account_cell_witness: PreAccountCellDataReader,
) -> Result<(), Error> {
    let price = u64::from(pre_account_cell_witness.price().new());
    let quote = u64::from(pre_account_cell_witness.quote());
    let discount = u32::from(pre_account_cell_witness.invited_discount());
    let duration = util::calc_duration_from_paid(profit, price, quote, discount);
    let expired_at = account_cell::get_expired_at(output_cell_data);
    let calculated_expired_at = current_timestamp + duration;

    debug!(
        "  Item[{}] Params of expired_at calculation: --profit={} --price={} --quote={} --discount={} --current={}",
        item_index, profit, price, quote, discount, current_timestamp
    );
    debug!(
        "  Item[{}] Critical value of expired_at calculation process: duration={}, calculated_expired_at={}",
        item_index, duration, calculated_expired_at
    );

    assert!(
        calculated_expired_at == expired_at,
        Error::ProposalConfirmNewAccountCellDataError,
        "  Item[{}] The AccountCell.expired_at should be {}, but {} found.",
        item_index,
        calculated_expired_at,
        expired_at
    );

    Ok(())
}

fn is_account_correct(item_index: usize, output_cell_data: &Vec<u8>) -> Result<(), Error> {
    let expected_account_id = account_cell::get_id(output_cell_data);
    let account = account_cell::get_account(output_cell_data);

    let hash = util::blake2b_256(account);
    let account_id = hash.get(..ACCOUNT_ID_LENGTH).unwrap();

    is_bytes_eq(
        item_index,
        "account",
        account_id,
        expected_account_id,
        Error::ProposalConfirmNewAccountCellDataError,
    )
}

fn is_cell_capacity_correct(
    item_index: usize,
    cell_index: usize,
    expected_capacity: u64,
) -> Result<(), Error> {
    let cell_capacity =
        load_cell_capacity(cell_index, Source::Output).map_err(|e| Error::from(e))?;

    assert!(
        expected_capacity == cell_capacity,
        Error::ProposalConfirmNewAccountCellCapacityError,
        "  Item[{}] The AccountCell.capacity should be {}, but {} found.",
        item_index,
        expected_capacity,
        cell_capacity
    );

    Ok(())
}

fn verify_witness_id(
    item_index: usize,
    output_cell_data: &Vec<u8>,
    output_cell_witness_reader: AccountCellDataReader,
) -> Result<(), Error> {
    let account_id = output_cell_witness_reader.id().raw_data();
    let expected_account_id = account_cell::get_id(output_cell_data);

    is_bytes_eq(
        item_index,
        "witness.id",
        account_id,
        expected_account_id,
        Error::ProposalConfirmWitnessIDError,
    )
}

fn verify_witness_account(
    item_index: usize,
    output_cell_data: &Vec<u8>,
    output_cell_witness_reader: AccountCellDataReader,
) -> Result<(), Error> {
    let mut account = output_cell_witness_reader.account().as_readable();
    account.append(&mut ACCOUNT_SUFFIX.as_bytes().to_vec());
    let expected_account = account_cell::get_account(output_cell_data);

    is_bytes_eq(
        item_index,
        "witness.account",
        account.as_slice(),
        expected_account,
        Error::ProposalConfirmWitnessAccountError,
    )
}

fn verify_witness_status(
    item_index: usize,
    output_cell_witness_reader: AccountCellDataReader,
) -> Result<(), Error> {
    let status = u8::from(output_cell_witness_reader.status());

    assert!(
        status == AccountStatus::Normal as u8,
        Error::ProposalConfirmWitnessManagerError,
        "  Item[{}] Check if outputs[].AccountCell.status is normal. (result: {}, expected: 0)",
        item_index,
        status
    );

    Ok(())
}

fn verify_refund_correct(
    proposal_cell_index: usize,
    proposal_cell_data_reader: ProposalCellDataReader,
) -> Result<(), Error> {
    debug!("Check if the refund amount to proposer_lock is correct.");

    let proposer_lock: Script = proposal_cell_data_reader.proposer_lock().to_entity();
    let refund_cells = util::find_cells_by_script(
        ScriptType::Lock,
        proposer_lock.as_reader().into(),
        Source::Output,
    )?;

    assert!(
        refund_cells.len() >= 1,
        Error::ProposalConfirmRefundError,
        "There should be at least 1 cell in outputs with the lock of the proposer. (expected_lock: {})",
        proposer_lock
    );

    let mut refund_capacity = 0;
    for index in refund_cells {
        refund_capacity += load_cell_capacity(index, Source::Output).map_err(|e| Error::from(e))?;
    }

    let proposal_capacity = load_cell_capacity(proposal_cell_index.to_owned(), Source::Input)
        .map_err(|e| Error::from(e))?;
    assert!(
        proposal_capacity <= refund_capacity,
        Error::ProposalConfirmRefundError,
        "There refund of proposer should be at least {}, but {} found.",
        proposal_capacity,
        refund_capacity
    );

    Ok(())
}
