import {
  CACHE_ACTION_PIN,
  CACHE_RESTORE_ACTION,
  CACHE_WRITER_ACTION,
  RUST_MAIN_CACHE_KEY,
  RUST_MAIN_CACHE_PREFIX,
  RUST_MAIN_WRITER_CACHE_KEY,
} from './cache-workflow-policy';

const cacheStep = (action: string, key: string): string => `      - name: Restore Cargo cache
        uses: ${action}
        with:
          path: target
          key: ${key}
          restore-keys: |
            ${RUST_MAIN_CACHE_PREFIX}
`;

export const compliantPrWorkflow = `name: RustTable PR\njobs:\n  validate:\n    steps:\n${cacheStep(CACHE_RESTORE_ACTION, RUST_MAIN_CACHE_KEY)}`;
export const compliantMainWorkflow = `name: RustTable Main\njobs:\n  full:\n    steps:\n${cacheStep(CACHE_WRITER_ACTION, RUST_MAIN_WRITER_CACHE_KEY)}`;

export const fixtures = {
  compliantPrWorkflow,
  compliantMainWorkflow,
  prWriter: compliantPrWorkflow.replace(CACHE_RESTORE_ACTION, CACHE_WRITER_ACTION),
  prSaver: `${compliantPrWorkflow}      - uses: actions/cache/save@${CACHE_ACTION_PIN}\n`,
  prWrongPrefix: compliantPrWorkflow.replace(`restore-keys: |\n            ${RUST_MAIN_CACHE_PREFIX}`, 'restore-keys: |\n            rust-pr-'),
  mainRestoreOnly: compliantMainWorkflow.replace(CACHE_WRITER_ACTION, CACHE_RESTORE_ACTION),
} as const;
