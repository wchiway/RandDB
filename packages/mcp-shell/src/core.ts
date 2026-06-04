import { spawn } from 'node:child_process';
import type { CoreRequest, CoreResult, TextToolResponse } from './contracts.js';

export interface CoreInvokerOptions {
  command?: string;
  args?: string[];
}

export async function invokeCore(
  request: CoreRequest,
  options: CoreInvokerOptions = {},
): Promise<CoreResult> {
  const command = options.command ?? process.env.RANDDB_CORE_BIN ?? 'randdb-cli';
  const args = options.args ?? [];
  const input = JSON.stringify(request);

  return new Promise((resolve) => {
    const child = spawn(command, args, { stdio: ['pipe', 'pipe', 'pipe'] });
    let stdout = '';
    let stderr = '';

    child.stdout.setEncoding('utf8');
    child.stderr.setEncoding('utf8');
    child.stdout.on('data', (chunk: string) => {
      stdout += chunk;
    });
    child.stderr.on('data', (chunk: string) => {
      stderr += chunk;
    });
    child.on('error', (error) => {
      resolve({
        status: 'error',
        error: {
          code: 'core_spawn_failed',
          message: error.message,
          operation: request.operation,
          retryable: false,
        },
      });
    });
    child.on('close', (code) => {
      if (code !== 0) {
        resolve({
          status: 'error',
          error: {
            code: 'core_exit_failed',
            message: stderr || `randdb core exited with code ${code}`,
            operation: request.operation,
            retryable: false,
          },
        });
        return;
      }

      try {
        resolve(JSON.parse(stdout) as CoreResult);
      } catch (error) {
        resolve({
          status: 'error',
          error: {
            code: 'core_invalid_json',
            message: error instanceof Error ? error.message : String(error),
            operation: request.operation,
            retryable: false,
          },
        });
      }
    });

    child.stdin.end(input);
  });
}

export function formatCoreResult(result: CoreResult): TextToolResponse {
  if (result.status === 'error') {
    return {
      content: [{ type: 'text', text: `Error: ${result.error?.message ?? 'unknown error'}` }],
      isError: true,
    };
  }

  return {
    content: [{ type: 'text', text: JSON.stringify(result.response ?? result, null, 2) }],
  };
}

export async function invokeCoreTool(
  request: CoreRequest,
  options?: CoreInvokerOptions,
): Promise<TextToolResponse> {
  return formatCoreResult(await invokeCore(request, options));
}
