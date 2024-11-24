/* tslint:disable */
/* eslint-disable */

/* auto-generated by NAPI-RS */

export interface TransformOptions {
  root: string
  output?: string
  externals?: Array<string>
  exclude?: Array<string>
  modules?: Array<string>
}
export interface PreOptimizeOptions {
  root: string
  output?: string
  packages: Array<string>
  externals?: Array<string>
  exclude?: Array<string>
  modules?: Array<string>
}
export declare function transform(options: TransformOptions): void
export declare function preOptimize(options: PreOptimizeOptions): void
