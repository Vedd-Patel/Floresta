# SPDX-License-Identifier: MIT OR Apache-2.0

"""
gettxout.py

This functional test cli utility to interact with a Floresta node with `gettxout` command.
"""

import pytest


# pylint: disable=too-many-locals
@pytest.mark.rpc
def test_get_txout(setup_logging, florestad_bitcoind_utreexod_with_chain, node_manager):
    """
    Test the `gettxout` command for a specific transaction output.
    """
    log = setup_logging
    blocks = 10
    florestad, bitcoind, _ = florestad_bitcoind_utreexod_with_chain(blocks)

    log.info("Waiting for Floresta and Bitcoind to sync with Utreexod...")
    node_manager.wait_for_sync_nodes()

    log.info("Comparing gettxout results between Floresta and Bitcoind...")
    for height in range(2, blocks):
        block_hash = florestad.rpc.get_blockhash(height)
        block = florestad.rpc.get_block(block_hash)
        log.info(f"Comparing gettxout results for {height} block {block_hash}...")

        for tx in block["tx"]:
            txout_floresta = florestad.rpc.get_txout(tx, vout=0, include_mempool=False)

            assert txout_floresta is not None, f"Txout for tx {tx} is None in Floresta."

            txout_bitcoind = bitcoind.rpc.get_txout(tx, vout=0, include_mempool=False)
            assert txout_bitcoind is not None, f"Txout for tx {tx} is None in Bitcoind."

            for key in txout_bitcoind.keys():
                if key in ["bestblock", "confirmations"]:
                    continue

                if key == "scriptPubKey":
                    for subkey in txout_bitcoind["scriptPubKey"].keys():
                        log.debug(f"Comparing scriptPubKey[{subkey}] for tx {tx}...")
                        assert (
                            txout_floresta["scriptPubKey"][subkey]
                            == txout_bitcoind["scriptPubKey"][subkey]
                        )
                else:
                    log.debug(f"Comparing {key} for tx {tx}...")
                    assert txout_floresta[key] == txout_bitcoind[key]
