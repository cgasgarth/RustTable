import {
  CACHE_ACTION_PIN,
  CACHE_WRITER_ACTION,
  RUST_MAIN_WRITER_CACHE_KEY,
} from './cache-workflow-policy';

const cacheStep = (action: string, key: string): string => `      - name: Restore Cargo cache
        uses: ${action}
        with:
          path: target
          key: ${key}
`;

export const compliantMainWorkflow = `name: RustTable Main\njobs:\n  full:\n    steps:\n${cacheStep(CACHE_WRITER_ACTION, RUST_MAIN_WRITER_CACHE_KEY)}`;

export const fixtures = {
  compliantMainWorkflow,
  mainRestoreOnly: compliantMainWorkflow.replace(CACHE_WRITER_ACTION, `actions/cache/restore@${CACHE_ACTION_PIN}`),
  mainMissingKey: compliantMainWorkflow.replace(`key: ${RUST_MAIN_WRITER_CACHE_KEY}`, 'key: stale'),
} as const;
