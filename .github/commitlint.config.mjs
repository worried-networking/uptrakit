export default {
  rules: {
    'type-enum': [2, 'always', [
      'feat', 'fix', 'docs', 'refactor', 'perf',
      'test', 'build', 'ci', 'chore',
      'style', 'revert', 'security',
    ]],
    'type-case': [2, 'always', 'lower-case'],
    'type-empty': [2, 'never'],
    'subject-empty': [2, 'never'],
    'subject-full-stop': [2, 'never', '.'],
    'header-max-length': [1, 'always', 100],
  },
  ignores: [(message) => /^Revert "/.test(message)],
};
