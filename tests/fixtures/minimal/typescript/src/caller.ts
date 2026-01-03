// Cross-file caller for testing find_callers across files
import { mainFunction, helper } from './index';

export function crossFileCaller(): void {
    mainFunction();
    helper();
}

export function anotherCaller(): void {
    crossFileCaller();
}
