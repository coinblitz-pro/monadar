declare const __BUILD_HASH__: string | undefined

export const BUILD_VERSION = typeof __BUILD_HASH__ !== 'undefined' ? __BUILD_HASH__ : 'dev'
