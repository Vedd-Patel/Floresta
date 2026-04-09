# SPDX-License-Identifier: MIT OR Apache-2.0

"""
floresta_cli_getblockheader.py

This functional test cli utility to interact with a Floresta node with `getblockheader`
"""

import re
import time

from test_framework import FlorestaTestFramework
from test_framework.node import NodeType


class GetBlockheaderHeightZeroTest(FlorestaTestFramework):
    """
    Test `getblockheader` with a fresh node and expect a result like this:

    ````bash
    $> ./target/release floresta_cli --network=regtest getblockheader \
        0f9188f13cb7b2c71f2a335e3a4fc328bf5beb436012afca590b1a11466e2206
    {
       "version": 1,
       "prev_blockhash": "0000000000000000000000000000000000000000000000000000000000000000",
       "merkle_root": "4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b",
       "time": 1296688602,
       "bits": 545259519,
       "nonce": 2
    }
    ```
    """

    version = 1
    blockhash = "0f9188f13cb7b2c71f2a335e3a4fc328bf5beb436012afca590b1a11466e2206"
    target = "7fffff0000000000000000000000000000000000000000000000000000000000"
    chainwork = "0000000000000000000000000000000000000000000000000000000000000002"
    n_tx = 1
    merkle_root = "4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b"
    time = 1296688602
    bits = "207fffff"
    nonce = 2

    v2transport = True

    def set_test_params(self):
        """
        Setup a single node
        """
        self.florestad = None
        self.bitcoind = None
        self.florestad = self.add_node_default_args(variant=NodeType.FLORESTAD)
        self.bitcoind = self.add_node_default_args(variant=NodeType.BITCOIND)

    def run_test(self):
        """
        Run JSONRPC and get the header of the genesis block
        """
        # Start nodes
        self.run_node(self.florestad)
        self.run_node(self.bitcoind)

        self.bitcoind.rpc.generate_block(2)

        self.connect_nodes(self.florestad, self.bitcoind)

        end = time.time() + 20
        while time.time() < end:
            if (
                self.florestad.rpc.get_block_count()
                == self.bitcoind.rpc.get_block_count()
            ):
                break
            time.sleep(0.5)

        # Test assertions
        response = self.florestad.rpc.get_blockheader(
            GetBlockheaderHeightZeroTest.blockhash
        )
        self.assertEqual(response["version"], GetBlockheaderHeightZeroTest.version)
        self.assertEqual(
            response["merkleroot"], GetBlockheaderHeightZeroTest.merkle_root
        )
        self.assertEqual(response["time"], GetBlockheaderHeightZeroTest.time)
        self.assertEqual(response["mediantime"], GetBlockheaderHeightZeroTest.time)
        self.assertEqual(response["bits"], GetBlockheaderHeightZeroTest.bits)
        self.assertEqual(response["target"], GetBlockheaderHeightZeroTest.target)
        self.assertEqual(response["chainwork"], GetBlockheaderHeightZeroTest.chainwork)
        self.assertEqual(response["nTx"], GetBlockheaderHeightZeroTest.n_tx)
        self.assertEqual(response["nonce"], GetBlockheaderHeightZeroTest.nonce)
        self.assertFalse("previousblockhash" in response)

        non_genesis_hash = self.florestad.rpc.get_blockhash(1)
        non_genesis_header = self.florestad.rpc.get_blockheader(non_genesis_hash)
        non_genesis_header_core = self.bitcoind.rpc.get_blockheader(non_genesis_hash)

        self.assertEqual(non_genesis_header["previousblockhash"], response["hash"])

        for key in [
            "bits",
            "difficulty",
            "mediantime",
            "target",
            "chainwork",
            "nTx",
            "previousblockhash",
        ]:
            if key == "difficulty":
                self.assertEqual(
                    round(non_genesis_header[key], 12),
                    round(non_genesis_header_core[key], 12),
                )
            else:
                self.assertEqual(non_genesis_header[key], non_genesis_header_core[key])

        non_genesis_header_hex = self.florestad.rpc.get_blockheader(
            non_genesis_hash, False
        )
        non_genesis_header_hex_core = self.bitcoind.rpc.get_blockheader(
            non_genesis_hash, False
        )
        self.assertTrue(bool(re.fullmatch(r"^[a-f0-9]{160}$", non_genesis_header_hex)))
        self.assertEqual(non_genesis_header_hex, non_genesis_header_hex_core)


if __name__ == "__main__":
    GetBlockheaderHeightZeroTest().main()
