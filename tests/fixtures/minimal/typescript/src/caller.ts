// Cross-file caller module for testing

import { mainFunction, helper } from './index';

export function crossFileCaller(): void {
    mainFunction();
    helper();
}

export function anotherCaller(): void {
    crossFileCaller();
}
