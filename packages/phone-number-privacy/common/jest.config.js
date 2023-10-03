module.exports = {
  preset: 'ts-jest',
  coverageReporters: [['lcov', { projectRoot: '../../../' }], 'text'],
  collectCoverageFrom: ['./src/**'],
  coverageThreshold: {
    global: {
      lines: 80,
    },
  },
  tsConfig: '<rootDir>/tsconfig.test.json',
}
