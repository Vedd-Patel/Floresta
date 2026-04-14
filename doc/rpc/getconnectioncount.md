# `getconnectioncount`

Returns the number of connections to other nodes we currently have.

## Usage

```bash
floresta-cli getconnectioncount
```

## Returns

### Ok Response

- `n` - (numeric) The number of active peer connections

### Error Response

- `Node` - Failed to retrieve connection count from the node

## Notes

- Only counts peers that have completed the handshake and are long-lived connections
- Feeler and extra connections are not included in the count
