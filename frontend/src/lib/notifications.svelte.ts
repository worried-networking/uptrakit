let successMessage: string | null = $state(null);
let errorMessage: string | null = $state(null);
let successTimer: ReturnType<typeof setTimeout> | null = null;

export function showSuccess(msg: string) {
	if (successTimer) clearTimeout(successTimer);
	successMessage = msg;
	successTimer = setTimeout(() => {
		successMessage = null;
		successTimer = null;
	}, 3000);
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
