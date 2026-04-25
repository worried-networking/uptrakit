let successMessage: string | null = $state(null);
let errorMessage: string | null = $state(null);

export function showSuccess(msg: string) {
	successMessage = msg;
}

export function clearSuccess() {
	successMessage = null;
}

export function showError(msg: string) {
	errorMessage = msg;
}

export function clearError() {
	errorMessage = null;
}

export function getSuccessMessage(): string | null {
	return successMessage;
}

export function getErrorMessage(): string | null {
	return errorMessage;
}
