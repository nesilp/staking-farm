use near_sdk::json_types::U128;
use near_sdk::serde_json::{self, json};
use near_sdk::AccountId;
use near_sdk_sim::{
    call, deploy, init_simulator, to_yocto, view, ContractAccount, ExecutionResult, UserAccount,
};

use staking_farm::{Ratio, StakingContractContract};

type PoolContract = ContractAccount<StakingContractContract>;

const STAKING_POOL_ACCOUNT_ID: &str = "pool";
const STAKING_KEY: &str = "KuTCtARNzxZQ3YvXDeLjx83FDqxv2SdQTSbiq876zR7";
const ONE_SEC_IN_NS: u64 = 1_000_000_000;
const TOKEN_ACCOUNT_ID: &str = "token";
const WHITELIST_ACCOUNT_ID: &str = "whitelist";
const LOCKUP_ACCOUNT_ID: &str = "lockup";

near_sdk_sim::lazy_static_include::lazy_static_include_bytes! {
    STAKING_FARM_BYTES => "../res/staking_farm_local.wasm",
    TEST_TOKEN_BYTES => "../res/test_token.wasm",
    WHITELIST_BYTES => "../res/whitelist.wasm",
    LOCKUP_BYTES => "../res/lockup_contract.wasm",
}

fn wait_epoch(user: &UserAccount) {
    let epoch_height = user.borrow_runtime().cur_block.epoch_height;
    while user.borrow_runtime().cur_block.epoch_height == epoch_height {
        assert!(user.borrow_runtime_mut().produce_block().is_ok());
    }
    // sim framework doesn't provide block rewards.
    // model the block reward by sending more funds on the account.
    user.transfer(
        AccountId::new_unchecked(STAKING_POOL_ACCOUNT_ID.to_string()),
        to_yocto("1000"),
    );
}

fn assert_all_success(result: ExecutionResult) {
    let mut all_success = true;
    let mut all_results = String::new();
    for r in result.promise_results() {
        let x = r.expect("NO_RESULT");
        all_results = format!("{}\n{:?}", all_results, x);
        all_success &= x.is_ok();
    }
    println!("{}", all_results);
    assert!(
        all_success,
        "Not all promises where successful: \n\n{}",
        all_results
    );
}

fn setup() -> (UserAccount, PoolContract) {
    let root = init_simulator(None);
    let _whitelist = root.deploy_and_init(
        &WHITELIST_BYTES,
        AccountId::new_unchecked(WHITELIST_ACCOUNT_ID.to_string()),
        "new",
        &serde_json::to_vec(&json!({ "foundation_account_id": root.account_id() })).unwrap(),
        to_yocto("10"),
        near_sdk_sim::DEFAULT_GAS,
    );
    let token_id = AccountId::new_unchecked(TOKEN_ACCOUNT_ID.to_string());
    let _other_token = root.deploy_and_init(
        &TEST_TOKEN_BYTES,
        token_id.clone(),
        "new",
        &[],
        to_yocto("10"),
        near_sdk_sim::DEFAULT_GAS,
    );
    assert_all_success(root.call(
        token_id.clone(),
        "mint",
        &serde_json::to_vec(&json!({ "account_id": root.account_id(), "amount": to_yocto("100000").to_string() })).unwrap(),
        near_sdk_sim::DEFAULT_GAS,
        0,
    ));
    let _lockup = root.deploy_and_init(
        &LOCKUP_BYTES,
        AccountId::new_unchecked(LOCKUP_ACCOUNT_ID.to_string()),
        "new",
        &serde_json::to_vec(&json!({ "owner_account_id": root.account_id(), "lockup_duration": "100000000000000", "transfers_information": { "TransfersEnabled": { "transfers_timestamp": "0" } }, "staking_pool_whitelist_account_id": WHITELIST_ACCOUNT_ID })).unwrap(),
        to_yocto("100000"),
        near_sdk_sim::DEFAULT_GAS,
    );
    let reward_ratio = Ratio {
        numerator: 1,
        denominator: 10,
    };
    let burn_ratio = Ratio {
        numerator: 3,
        denominator: 10,
    };
    let pool = deploy!(
        contract: StakingContractContract,
        contract_id: STAKING_POOL_ACCOUNT_ID.to_string(),
        bytes: &STAKING_FARM_BYTES,
        signer_account: root,
        deposit: to_yocto("5"),
        init_method: new(root.account_id(), STAKING_KEY.parse().unwrap(), reward_ratio, burn_ratio)
    );
    assert_all_success(root.call(
        token_id,
        "storage_deposit",
        &serde_json::to_vec(&json!({ "account_id": pool.account_id() })).unwrap(),
        near_sdk_sim::DEFAULT_GAS,
        to_yocto("1"),
    ));
    (root, pool)
}

#[test]
fn test_farm_and_burn() {
    let (root, pool) = setup();
    let user1 = root.create_user(
        AccountId::new_unchecked("user1".to_string()),
        to_yocto("100000"),
    );
    assert_all_success(call!(
        user1,
        pool.deposit_and_stake(),
        deposit = to_yocto("10000")
    ));
    wait_epoch(&root);
    assert_all_success(call!(root, pool.ping()));
    // Check that out of 1000 reward, 300 has burnt, 10% went to root, leaving ~630.
    let balance0 = view!(pool.get_account_total_balance(root.account_id())).unwrap_json::<U128>();
    assert!(balance0.0 > to_yocto("69") && balance0.0 < to_yocto("71"));
    let balance1 = view!(pool.get_account_total_balance(user1.account_id())).unwrap_json::<U128>();
    assert!(balance1.0 > to_yocto("10629") && balance1.0 < to_yocto("10630"));

    // Deploy a farm.
    let start_date = root.borrow_runtime().cur_block.block_timestamp + ONE_SEC_IN_NS * 3;
    let end_date = start_date + ONE_SEC_IN_NS * 5;
    let msg =
        serde_json::to_string(&json!({ "name": "Test", "start_date": format!("{}", start_date), "end_date": format!("{}", end_date) }))
            .unwrap();
    assert_all_success(root.call(
        AccountId::new_unchecked(TOKEN_ACCOUNT_ID.to_string()),
        "ft_transfer_call",
        &serde_json::to_vec(&json!({ "receiver_id": STAKING_POOL_ACCOUNT_ID, "amount": to_yocto("50000").to_string(), "msg": msg })).unwrap(),
        near_sdk_sim::DEFAULT_GAS,
        1
    ));

    let other_balance1 = view!(pool.get_unclaimed_reward(user1.account_id(), 0))
        .unwrap_json::<U128>()
        .0;
    assert!(
        other_balance1 > to_yocto("19000") && other_balance1 < to_yocto("20000"),
        "{}",
        other_balance1
    );

    root.borrow_runtime_mut().produce_block().unwrap();

    let other_balance2 = view!(pool.get_unclaimed_reward(user1.account_id(), 0))
        .unwrap_json::<U128>()
        .0;
    assert!(
        other_balance2 > to_yocto("29000") && other_balance2 < to_yocto("30000"),
        "{}",
        other_balance2
    );

    for _ in 0..2 {
        root.borrow_runtime_mut().produce_block().unwrap();
    }

    // After 5 blocks passed, all rewards were mined.
    // The split is 10670 / 10700 went to user1, and 70 / 10700 went to root.
    // Because root received already it's reward from staking as pool owner and also participates in farming.
    let other_balance3 = view!(pool.get_unclaimed_reward(user1.account_id(), 0))
        .unwrap_json::<U128>()
        .0;
    let other_balance4 = view!(pool.get_unclaimed_reward(root.account_id(), 0))
        .unwrap_json::<U128>()
        .0;
    assert!(
        other_balance3 > to_yocto("49640") && other_balance3 < to_yocto("49650"),
        "{}",
        other_balance3
    );
    assert!(other_balance4 > to_yocto("325") && other_balance4 < to_yocto("327"));
}