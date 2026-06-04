import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { invokeCore } from './core.js';
import type { CoreRequest } from './contracts.js';

const cargoArgs = ['run', '-q', '-p', 'randdb-cli'];

async function invokeRustCore(request: CoreRequest) {
  return invokeCore(request, { command: 'cargo', args: cargoArgs });
}

describe('Rust core boundary', () => {
  it('round-trips index, list-files, stats, and health through the real Rust CLI JSON contract', async () => {
    const root = await mkdtemp(join(tmpdir(), 'randdb-core-'));
    try {
      await mkdir(join(root, 'src'));
      await writeFile(
        join(root, 'src', 'lib.rs'),
        'pub fn source() {}\nfn caller() { source(); }\nfn sourceful() {}\n',
      );

      const indexResult = await invokeRustCore({
        operation: 'index-repository',
        payload: { repo_path: root, force_rebuild: true },
      });
      expect(indexResult.status).toBe('ok');
      expect(indexResult.response).toMatchObject({
        operation: 'index-repository',
        payload: {
          project_id: expect.stringMatching(/^randdb-core-/),
          run: {
            scanned_files: 1,
            added_files: 1,
            modified_files: 0,
            deleted_files: 0,
          },
        },
      });

      const listResult = await invokeRustCore({
        operation: 'list-files',
        payload: { repo_path: root, glob: 'src/*.rs', language: 'rust', max_results: 10 },
      });
      expect(listResult.status).toBe('ok');
      expect(listResult.response).toEqual({
        operation: 'list-files',
        payload: {
          files: [
            {
              path: 'src/lib.rs',
              language: 'rust',
              size: expect.any(Number),
              hash: expect.any(String),
            },
          ],
          truncated: false,
        },
      });

      const retrievalResult = await invokeRustCore({
        operation: 'codebase-retrieval',
        payload: {
          repo_path: root,
          information_request: 'Find source function',
          technical_terms: ['source'],
          mode: 'quick',
          include_globs: ['src/*.rs'],
          language: ['rust'],
          output_format: 'both',
          return_debug: true,
        },
      });
      expect(retrievalResult.status).toBe('ok');
      expect(retrievalResult.response).toMatchObject({
        operation: 'codebase-retrieval',
        payload: {
          format: 'both',
          markdown: expect.stringContaining('source'),
          pack: {
            files: [
              {
                path: 'src/lib.rs',
              },
            ],
          },
          debug: {
            lexical_query: 'source',
            semantic_query: 'Find source function',
          },
        },
      });

      const definitionResult = await invokeRustCore({
        operation: 'get-symbol-definition',
        payload: { repo_path: root, symbol: 'source', hint_path: 'src/lib.rs', max_results: 5 },
      });
      expect(definitionResult.status).toBe('ok');
      expect(definitionResult.response).toMatchObject({
        operation: 'get-symbol-definition',
        payload: {
          symbol: 'source',
          definitions: [
            {
              path: 'src/lib.rs',
              language: 'rust',
              start_line: 1,
              end_line: 1,
              text: 'pub fn source() {}',
            },
          ],
          truncated: false,
        },
      });

      const referencesResult = await invokeRustCore({
        operation: 'find-references',
        payload: { repo_path: root, symbol: 'source', exclude_definition: true, max_results: 10 },
      });
      expect(referencesResult.status).toBe('ok');
      expect(referencesResult.response).toMatchObject({
        operation: 'find-references',
        payload: {
          symbol: 'source',
          references: [
            {
              path: 'src/lib.rs',
              language: 'rust',
              start_line: 2,
              end_line: 2,
              snippet: 'fn caller() { source(); }',
            },
          ],
          truncated: false,
        },
      });

      const statsResult = await invokeRustCore({
        operation: 'stats',
        payload: { repo_path: root },
      });
      expect(statsResult.status).toBe('ok');
      expect(statsResult.response).toMatchObject({
        operation: 'stats',
        payload: {
          health: {
            file_count: 1,
            chunk_count: 1,
          },
          index: {
            total_runs: 1,
          },
        },
      });

      const healthResult = await invokeRustCore({
        operation: 'health',
        payload: { repo_path: root },
      });
      expect(healthResult.status).toBe('ok');
      expect(healthResult.response).toMatchObject({
        operation: 'health',
        payload: {
          health: {
            file_count: 1,
            chunk_count: 1,
          },
        },
      });
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);
});
