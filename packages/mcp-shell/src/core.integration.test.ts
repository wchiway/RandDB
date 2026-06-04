import { describe, expect, it } from 'vitest';
import { invokeCore } from './core.js';

const cargoArgs = ['run', '-q', '-p', 'randdb-cli'];

describe('Rust core boundary', () => {
  it('round-trips MCP shell requests through the real Rust CLI JSON contract', async () => {
    const result = await invokeCore(
      {
        operation: 'stats',
        payload: { repo_path: '/repo' },
      },
      { command: 'cargo', args: cargoArgs },
    );

    expect(result).toEqual({
      status: 'error',
      error: {
        code: 'unsupported_operation',
        message: 'operation is not implemented by this RandDB core build',
        operation: 'stats',
        retryable: false,
      },
    });
  }, 30_000);
});
