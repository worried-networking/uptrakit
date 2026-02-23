let isOnline = $state(true); // Default to online

function updateOnlineStatus() {
	isOnline = navigator.onLine;
}

export function getIsOnline(): boolean {
	return isOnline;
}

// Check initial status
if (typeof window !== 'undefined') {
	updateOnlineStatus(); // Set initial status
	// These listeners are intentional module-level singletons: they live for the full
	// app lifetime, so no cleanup function is needed (the module is never reloaded).
	window.addEventListener('online', updateOnlineStatus);
	window.addEventListener('offline', updateOnlineStatus);
}
