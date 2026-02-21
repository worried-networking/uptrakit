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
	window.addEventListener('online', updateOnlineStatus);
	window.addEventListener('offline', updateOnlineStatus);
}
