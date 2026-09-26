// ABOUTME: Babel configuration for Expo with NativeWind v4 support and the React Compiler
// ABOUTME: babel-preset-expo adds the react-native-worklets plugin itself whenever the package is installed

module.exports = function(api) {
  api.cache(true);
  return {
    presets: [
      ['babel-preset-expo', { jsxImportSource: 'nativewind' }],
    ],
    plugins: [
      'babel-plugin-react-compiler',
    ],
  };
};
