// ABOUTME: Stands in for Metro's `expo/virtual/env` module so app e2e specs can import real source files.
// ABOUTME: babel-preset-expo rewrites every `process.env.X` read to this module; Metro supplies it on device.

module.exports = { env: process.env };
