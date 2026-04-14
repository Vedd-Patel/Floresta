# SPDX-License-Identifier: MIT OR Apache-2.0

"""
floresta_cli_getconnectioncount.py

This functional test exercises the `getconnectioncount` RPC by starting
a Floresta node and multiple bitcoind nodes, then verifying the connection
count as peers are progressively connected. Also compares behavior against
bitcoind's own getconnectioncount for parity.
"""

from test_framework import FlorestaTestFramework
from test_framework.node import NodeType


class GetConnectionCountTest(FlorestaTestFramework):
    """
    Test `getconnectioncount` by connecting multiple bitcoind peers
    and verifying the count increases accordingly. Also compares
    behavior against bitcoind's own getconnectioncount.
    """

    def set_test_params(self):
        self.florestad = self.add_node_default_args(variant=NodeType.FLORESTAD)
        self.bitcoind = self.add_node_default_args(variant=NodeType.BITCOIND)
        self.peers = [
            self.add_node_default_args(variant=NodeType.BITCOIND) for _ in range(5)
        ]

    def run_test(self):
        # Start florestad and bitcoind with shared peers, connect them
        # progressively, and compare getconnectioncount results.
        self.run_node(self.florestad)
        self.run_node(self.bitcoind)
        for peer in self.peers:
            self.run_node(peer)

        # Starts at 0
        self.assertEqual(self.florestad.rpc.get_connectioncount(), 0)

        # Direct comparison — connect florestad to bitcoind, both should report 1
        self.connect_nodes(self.florestad, self.bitcoind)

        floresta_count = self.florestad.rpc.get_connectioncount()
        bitcoind_count = self.bitcoind.rpc.get_connectioncount()
        self.log(f"florestad={floresta_count}, bitcoind={bitcoind_count}")
        self.assertEqual(floresta_count, bitcoind_count)

        # Connect each peer and verify count increments by 1
        for i, peer in enumerate(self.peers):
            self.connect_nodes(self.florestad, peer)
            self.assertEqual(self.florestad.rpc.get_connectioncount(), i + 2)

        # Disconnect bitcoind and verify count decreases
        self.florestad.rpc.disconnectnode(self.bitcoind.p2p_url)
        self.wait_for_peers_connections(
            self.florestad, self.bitcoind, is_connected=False
        )
        self.assertEqual(self.florestad.rpc.get_connectioncount(), 5)


if __name__ == "__main__":
    GetConnectionCountTest().main()
