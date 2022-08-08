// Copyright 2020-2022 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

import {
    IStardustIdentityClient,
    IStardustIdentityClientExt,
    StardustDID,
    StardustDocument,
    StardustIdentityClientExt
} from '../../node';

import {
    ADDRESS_UNLOCK_CONDITION_TYPE,
    ALIAS_OUTPUT_TYPE,
    BASIC_OUTPUT_TYPE,
    Bech32Helper,
    DEFAULT_PROTOCOL_VERSION,
    ED25519_ADDRESS_TYPE,
    ED25519_SIGNATURE_TYPE,
    Ed25519Address,
    IAliasOutput,
    IBasicOutput,
    IBlock,
    IClient,
    IKeyPair,
    IndexerPluginClient,
    IOutputsResponse,
    IRent,
    ISignatureUnlock,
    ITransactionEssence,
    ITransactionPayload,
    IUTXOInput,
    OutputTypes,
    promote,
    reattach,
    serializeOutput,
    SIGNATURE_UNLOCK_TYPE,
    TRANSACTION_ESSENCE_TYPE,
    TRANSACTION_PAYLOAD_TYPE,
    TransactionHelper
} from '@iota/iota.js';
import {Converter, WriteStream} from "@iota/util.js";
import {Blake2b, Ed25519} from "@iota/crypto.js";

/** Provides operations for IOTA UTXO DID Documents with Alias Outputs. */
export class StardustIdentityClient implements IStardustIdentityClient, IStardustIdentityClientExt {
    client: IClient;
    indexer: IndexerPluginClient;

    constructor(client: IClient) {
        this.client = client;
        this.indexer = new IndexerPluginClient(client);
    }

    async getNetworkHrp() {
        const nodeInfo = await this.client.protocolInfo();
        return nodeInfo.bech32Hrp;
    }

    async getAliasOutput(aliasId: string) {
        // Lookup latest OutputId from the indexer plugin.
        const aliasResponse = await this.indexer.alias(aliasId);
        if (aliasResponse.items.length == 0) {
            throw new Error("AliasId '" + aliasId + "' not found");
        }
        const outputId = aliasResponse.items[0];

        // Fetch AliasOutput.
        const outputResponse = await this.client.output(outputId);
        const output = outputResponse.output;
        if (output.type != ALIAS_OUTPUT_TYPE) {
            throw new Error("AliasId '" + aliasId + "' returned incorrect type '" + output.type + "'");
        }
        // Coerce to tuple instead of an array.
        const ret: [string, IAliasOutput] = [outputId, output];
        return ret;
    }

    async getRentStructure() {
        const nodeInfo = await this.client.info();
        return nodeInfo.protocol.rentStructure;
    }

    async newDidOutput(addressType: number, addressHex: string, document: StardustDocument, rentStructure?: IRent): Promise<IAliasOutput> {
        return await StardustIdentityClientExt.newDidOutput(this, addressType, addressHex, document, rentStructure);
    }

    async updateDidOutput(document: StardustDocument): Promise<IAliasOutput> {
        return await StardustIdentityClientExt.updateDidOutput(this, document);
    }

    async deactivateDidOutput(did: StardustDID): Promise<IAliasOutput> {
        return await StardustIdentityClientExt.deactivateDidOutput(this, did);
    }

    async resolveDid(did: StardustDID): Promise<StardustDocument> {
        return await StardustIdentityClientExt.resolveDid(this, did);
    }

    async resolveDidOutput(did: StardustDID): Promise<IAliasOutput> {
        return await StardustIdentityClientExt.resolveDidOutput(this, did);
    }

    async publishDidOutput(walletKeyPair: IKeyPair, aliasOutput: IAliasOutput): Promise<StardustDocument> {
        const networkHrp = await this.getNetworkHrp();

        const consumedOutputs: [string, OutputTypes][] = [];
        const outputs: OutputTypes[] = [aliasOutput];

        // Check if tokens need to be transferred to or from the output.
        let consumeAmount: bigint = BigInt(0);
        let remainderAmount: bigint = BigInt(0);
        if (aliasOutput.stateIndex === 0) {
            consumeAmount = BigInt(aliasOutput.amount);
        } else {
            const previousAlias: [string, IAliasOutput] = await this.getAliasOutput(aliasOutput.aliasId);
            const previousAmount = BigInt(previousAlias[1].amount);
            const nextAmount = BigInt(aliasOutput.amount)
            if (nextAmount > previousAmount) {
                consumeAmount = previousAmount - nextAmount;
            } else {
                remainderAmount = nextAmount - previousAmount;
            }

            // Consume previous Alias Output.
            consumedOutputs.push(previousAlias);
        }

        // Get the wallet address, which is the Blake2b-256 digest of the public key.
        const walletEd25519Address = new Ed25519Address(walletKeyPair.publicKey);
        const walletAddress = walletEd25519Address.toAddress();
        if (consumeAmount > BigInt(0)) {
            // Get tokens from wallet.
            const walletAddressBech32 = Bech32Helper.toBech32(ED25519_ADDRESS_TYPE, walletAddress, networkHrp);
            const walletOutput: [string, IBasicOutput] = await fetchBasicOutputWithAmount(walletAddressBech32, consumeAmount, this);
            // Mark any excess funds for return.
            if (BigInt(walletOutput[1].amount) > consumeAmount) {
                remainderAmount = remainderAmount + BigInt(walletOutput[1].amount) - consumeAmount;
            }

            // Consume wallet output.
            consumedOutputs.push(walletOutput);
        }

        // Send remainder tokens to wallet.
        if (remainderAmount > BigInt(0)) {
            const walletAddressHex = Converter.bytesToHex(walletAddress, true);
            const walletOutput: IBasicOutput = {
                type: BASIC_OUTPUT_TYPE,
                amount: remainderAmount.toString(),
                nativeTokens: [],
                unlockConditions: [
                    {
                        type: ADDRESS_UNLOCK_CONDITION_TYPE,
                        address: {
                            type: ED25519_ADDRESS_TYPE,
                            pubKeyHash: walletAddressHex
                        }
                    }
                ],
                features: []
            };
            outputs.push(walletOutput);
        }

        // Compute transaction essence from outputs.
        const essence: ITransactionEssence = await prepareTransactionEssence(this.client, consumedOutputs, outputs);

        // Compute Transaction Essence Hash (to be signed in signature unlocks).
        const essenceHash = TransactionHelper.getTransactionEssenceHash(essence);

        // We unlock only one output, so there will be one unlock with signature.
        let unlock: ISignatureUnlock = {
            type: SIGNATURE_UNLOCK_TYPE,
            signature: {
                type: ED25519_SIGNATURE_TYPE,
                publicKey: Converter.bytesToHex(walletKeyPair.publicKey, true),
                signature: Converter.bytesToHex(Ed25519.sign(walletKeyPair.privateKey, essenceHash), true)
            }
        };

        // Constructing Transaction Payload.
        const txPayload: ITransactionPayload = {
            type: TRANSACTION_PAYLOAD_TYPE,
            essence: essence,
            unlocks: [unlock]
        };

        // Get parents for the block proof-of-work.
        let parentsResponse = await this.client.tips();
        let parents = parentsResponse.tips;

        // Construct block containing the transaction.
        let block: IBlock = {
            protocolVersion: DEFAULT_PROTOCOL_VERSION,
            parents: parents,
            payload: txPayload,
            nonce: "0"
        };

        // Extract document with computed AliasId.
        const documents = extractDocumentsFromPayload(networkHrp, txPayload);
        if (documents.length < 1) {
            throw new Error("publishDidOutput: no DID document in transaction payload, aborting publishing");
        }

        // Publish the block.
        const blockId = await this.client.blockSubmit(block);
        await retryUntilIncluded(this.client, blockId, 5000, 20);

        // Checked for non-zero length above.
        return documents[0];
    }

    /// TODO: helper functions for deletion.
}

async function prepareTransactionEssence(client: IClient, consumedOutputs: [string, OutputTypes][], outputs: OutputTypes[]): Promise<ITransactionEssence> {
    const inputs: IUTXOInput[] = [];
    for (const consumedOutput of consumedOutputs) {
        const input: IUTXOInput = TransactionHelper.inputFromOutputId(consumedOutput[0]);
        inputs.push(input);
    }

    // Compute inputs commitment.
    const inputsCommitmentHasher = new Blake2b(Blake2b.SIZE_256);
    // Hash list of inputs (the actual output objects they reference).
    const outputHasher = new Blake2b(Blake2b.SIZE_256);
    const w = new WriteStream();
    for (const consumedOutput of consumedOutputs) {
        serializeOutput(w, consumedOutput[1]);
        const consumedOutputBytes = w.finalBytes();
        outputHasher.update(consumedOutputBytes);
        const outputHash = outputHasher.final();

        inputsCommitmentHasher.update(outputHash);
    }
    // Calculate sum from buffer.
    const inputsCommitment: string = Converter.bytesToHex(inputsCommitmentHasher.final(), true);

    // Creating Transaction Essence
    const protocolInfo = await client.protocolInfo();
    return {
        type: TRANSACTION_ESSENCE_TYPE,
        networkId: protocolInfo.networkId,
        inputs,
        outputs,
        inputsCommitment,
    }
}

/** Promotes or re-attaches the given block id until it's included (referenced by a milestone).
 *
 *  This is copied as closely as possible from the `iota.rs` implementation:
 *  https://github.com/iotaledger/iota.rs/blob/128283b14e6476b2fc497d2e4fd27028277a3a59/src/client.rs#L529
 */
async function retryUntilIncluded(client: IClient, blockId: string, intervalMs: number, maxAttempts: number) {
    // Track blocks, since re-attaching a block might produce more than one.
    const blockIds: string[] = [blockId];

    for (let attempt = 0; attempt < maxAttempts; attempt += 1) {
        // Sleep.
        await new Promise(f => setTimeout(f, intervalMs));

        const length = blockIds.length;
        for (let index = 0; index < length; index += 1) {
            const currentBlockId = blockIds[index];

            // Check if block is already included successfully.
            const metadata = await client.blockMetadata(currentBlockId);
            if (metadata.ledgerInclusionState === "included" || metadata.ledgerInclusionState === "noTransaction") {
                return;
            }

            // Only promote or re-attach the latest attachment of the block.
            if (index == blockIds.length - 1) {
                if (metadata.shouldPromote) {
                    await promote(client, currentBlockId);
                } else if (metadata.shouldReattach) {
                    const reattached = await reattach(client, currentBlockId);
                    // Only track new reattached blocks; promoted blocks are empty and just attempt to confirm the
                    // original block.
                    blockIds.push(reattached.blockId);
                }
            }
        }
    }
}

/** Extract all DID documents of the Alias Outputs contained in a transaction payload, if any. */
function extractDocumentsFromPayload(networkHrp: string, payload: ITransactionPayload): StardustDocument[] {
    const documents: StardustDocument[] = [];

    // Compute TransactionId.
    const transactionPayloadHash: Uint8Array = TransactionHelper.getTransactionPayloadHash(payload);
    const transactionId: string = Converter.bytesToHex(transactionPayloadHash, true);

    // Loop over Alias Outputs.
    const outputs: OutputTypes[] = payload.essence.outputs;
    for (let index = 0; index < outputs.length; index += 1) {
        const output = outputs[index];
        if (output.type !== ALIAS_OUTPUT_TYPE) {
            continue;
        }

        // Compute Alias Id.
        let aliasId: Uint8Array;
        if (output.stateIndex === 0) {
            const outputIdHex: string = TransactionHelper.outputIdFromTransactionData(transactionId, index);
            const outputIdBytes: Uint8Array = Converter.hexToBytes(outputIdHex);
            aliasId = Blake2b.sum256(outputIdBytes);
        } else {
            const aliasIdHex: string = output.aliasId;
            aliasId = Converter.hexToBytes(aliasIdHex);
        }

        // Unpack document.
        const did: StardustDID = new StardustDID(aliasId, networkHrp);
        let stateMetadata: Uint8Array;
        if (output.stateMetadata === undefined) {
            stateMetadata = new Uint8Array(0);
        } else {
            stateMetadata = Converter.hexToBytes(output.stateMetadata);
        }
        documents.push(StardustDocument.unpack(did, stateMetadata, true));
    }
    return documents;
}

/** Attempt to fetch a Basic Output with at least the minimum specified token amount.
 *
 *  If multiple blocks satisfy the minimum, returns the block that matches exactly, or else the one with the largest
 *  amount (to try to avoid creating an output below the dust threshold).
 */
// TODO: allow selecting multiple small outputs to consume? Ideally the developer should consolidate funds
//       or prefer `iota-client` (@iota/client) for Node.js which does this for us.
async function fetchBasicOutputWithAmount(addressBech32: string, minAmount: bigint, didClient: StardustIdentityClient): Promise<[string, IBasicOutput]> {
    // Fetch all Basic Output ids from indexer plugin.
    let outputsResponse: IOutputsResponse = {ledgerIndex: 0, cursor: "", pageSize: "", items: []};
    let maxTries = 5;
    let tries = 0;
    while (outputsResponse.items.length == 0) {
        if (tries > maxTries) {
            break;
        }
        tries++;
        outputsResponse = await didClient.indexer.outputs({
            addressBech32: addressBech32,
            hasStorageReturnCondition: false,
            hasExpirationCondition: false,
            hasTimelockCondition: false,
            hasNativeTokens: false,
        });
        if (outputsResponse.items.length == 0) {
            await new Promise(f => setTimeout(f, 1000));
        }
    }
    if (tries > maxTries) {
        throw new Error("failed to find any Basic Outputs for address " + addressBech32);
    }

    // Fetch all Basic Outputs from client.
    const basicOutputs: [string, IBasicOutput][] = [];
    for (const outputId of outputsResponse.items) {
        const basicOutput: OutputTypes = (await didClient.client.output(outputId)).output;
        if (basicOutput.type !== BASIC_OUTPUT_TYPE) {
            continue;
        }
        basicOutputs.push([outputId, basicOutput]);
    }

    // Select Basic Output matching amount exactly, otherwise one with the largest amount.
    let matchOutput: [string, IBasicOutput] | null = null;
    for (const [outputId, output] of basicOutputs) {
        const outputAmount = BigInt(output.amount);
        if (outputAmount === minAmount) {
            // Exact match.
            matchOutput = [outputId, output];
            break;
        } else if (outputAmount > minAmount && (matchOutput == null || BigInt(matchOutput[1].amount) < outputAmount)) {
            // Largest amount.
            matchOutput = [outputId, output];
        }
    }
    if (matchOutput === null) {
        throw new Error("failed to find a Basic Output with at least " + minAmount + " tokens, consolidate or deposit more tokens for address " + addressBech32);
    }
    return matchOutput;
}
