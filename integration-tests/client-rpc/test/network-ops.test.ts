import "mocha";
import chaiAsPromised = require("chai-as-promised");
import { use as chaiUse, expect } from "chai";
import { RpcClient } from "./core/rpc-client";
import {
	generateWalletName,
	newWalletRequest,
	unbondAndWithdrawStake,
	newZeroFeeRpcClient,
	WALLET_STAKING_ADDRESS,
	sleep,
} from "./core/setup";
import BigNumber from "bignumber.js";
chaiUse(chaiAsPromised);

describe.only("Staking", () => {
	let client: RpcClient;
	before(async () => {
		await unbondAndWithdrawStake();
		client = newZeroFeeRpcClient();
	});

	it("should support staking, unbonding and withdrawing", async () => {
        const defaultWalletRequest = newWalletRequest("Default", "123456");

		const walletName = generateWalletName();
        const walletRequest = newWalletRequest(walletName, "123456");
		await client.request("wallet_create", [
			walletRequest,
		]);
		const stakingAddress = await client.request("wallet_createStakingAddress", [
			walletRequest,
		]);
		const transferAddress = await client.request("wallet_createTransferAddress", [
			walletRequest,
		]);
        const viewKey = await client.request("wallet_getViewKey", [walletRequest]);
        
        console.log(walletName, stakingAddress, transferAddress, viewKey);

		const stakingAmount = "1000";
		let txId = await client.request("wallet_sendToAddress", [
			defaultWalletRequest,
			transferAddress,
			stakingAmount,
			[viewKey],
		]);

		await sleep(2000);

		client.request("sync", [walletRequest]);
		expect(
			client.request("wallet_balance", [walletRequest]),
        ).to.eventually.deep.eq(stakingAmount);
        
        console.log(txId);

		await expect(
			client.request("staking_depositStake", [
				walletRequest,
				stakingAddress,
				[
					{
						id: txId,
						index: 0,
					},
				],
			]),
		).to.eventually.eq(null, "Deposit stake should work");
		const stakingStateAfterDeposit = await client.request("staking_state", [
			walletRequest,
			stakingAddress,
		]);
		assertStakingState(
			stakingStateAfterDeposit,
			{
				address: stakingAddress,
				bonded: stakingAmount,
				unbonded: "0",
			},
			"Staking state is incorrect after deposit stake",
		);
		await sleep(2000);
        client.request("sync", [walletRequest]);
		expect(
			client.request("wallet_balance", [walletRequest]),
		).to.eventually.deep.eq(stakingAmount, "Wallet balance should be deducted after deposit stake");

		const unbondAmount = "500";
		const remainingBondedAmount = new BigNumber(stakingAmount)
			.minus(unbondAmount)
			.toString(10);
		await expect(
			client.request("staking_unbondStake", [
				walletRequest,
				stakingAddress,
				unbondAmount,
			]),
		).to.eventually.eq(null, "Unbond stake should work");
		const stakingStateAfterUnbond = await client.request("staking_state", [
			walletRequest,
			stakingAddress,
		]);
		assertStakingState(
			stakingStateAfterUnbond,
			{
				address: stakingAddress,
				bonded: remainingBondedAmount,
				unbonded: unbondAmount,
			},
			"Staking state is incorrect after unbond stake",
		);

		await expect(
			client.request("staking_withdrawAllUnbondedStake", [
				walletRequest,
				stakingAddress,
				transferAddress,
				[],
			]),
		).to.eventually.eq(null, "Unbond stake should work");
		const stakingStateAfterWithdraw = await client.request("staking_state", [
			walletRequest,
			WALLET_STAKING_ADDRESS,
		]);
		assertStakingState(
			stakingStateAfterWithdraw,
			{
				address: WALLET_STAKING_ADDRESS,
				bonded: remainingBondedAmount,
				unbonded: "0",
			},
			"Staking state is incorrect after withdraw stake",
		);
		await sleep(2000);
        client.request("sync", [walletRequest]);
		expect(
			client.request("wallet_balance", [walletRequest]),
		).to.eventually.deep.eq(stakingAmount, "Wallet balance should be credited after withdraw stake");
	});

	const assertStakingState = (
		actualState: StakingState,
		expectedState: Omit<StakingState, "unbonded_from">,
		errorMessage: string = "Staking state does not match",
	) => {
		Object.keys(expectedState).forEach((prop) => {
			expect(expectedState[prop]).to.deep.eq(actualState[prop], errorMessage);
		});
    };
    
    type Omit<T, K> = Pick<T, Exclude<keyof T, K>>;

	interface StakingState {
		address?: string;
		bonded?: string;
		nonce?: number;
		unbonded?: string;
		unbonded_from: number;
	}
});
