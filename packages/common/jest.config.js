module.exports = {
  preset: 'ts-jest',
  coverageReporters: [['lcov', { projectRoot: '../../../' }], 'text'],
  collectCoverageFrom: ['./src/**'],
  coverageThreshold: {
    global: {
      lines: 80,
    },
  },
  transform: {
    '^.+\\.tsx?$': ['ts-jest', {
      tsconfig: '<rootDir>/tsconfig.test.json',
    }],
  },
}
