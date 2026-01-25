// Aelys HTTP Server - Client-side JavaScript

// Fetch and display API response
async function fetchAPI(endpoint) {
    const responseElement = document.getElementById('response');
    responseElement.textContent = 'Loading...';

    try {
        const response = await fetch(endpoint);
        const data = await response.json();

        // Pretty print JSON
        responseElement.textContent = JSON.stringify(data, null, 2);

        // Add success indicator
        if (response.ok) {
            responseElement.style.borderLeft = '4px solid #10b981';
        } else {
            responseElement.style.borderLeft = '4px solid #ef4444';
        }
    } catch (error) {
        responseElement.textContent = `Error: ${error.message}`;
        responseElement.style.borderLeft = '4px solid #ef4444';
    }
}

// Test POST /api/echo endpoint
async function testEcho() {
    const responseElement = document.getElementById('response');
    responseElement.textContent = 'Sending POST request...';

    const testMessage = 'Hello from Aelys HTTP Server!';

    try {
        const response = await fetch('/api/echo', {
            method: 'POST',
            headers: {
                'Content-Type': 'text/plain'
            },
            body: testMessage
        });

        const data = await response.json();

        // Pretty print JSON
        responseElement.textContent = JSON.stringify(data, null, 2);

        // Add success indicator
        if (response.ok) {
            responseElement.style.borderLeft = '4px solid #10b981';
        } else {
            responseElement.style.borderLeft = '4px solid #ef4444';
        }
    } catch (error) {
        responseElement.textContent = `Error: ${error.message}`;
        responseElement.style.borderLeft = '4px solid #ef4444';
    }
}

// Add keyboard shortcuts
document.addEventListener('keydown', (e) => {
    // Ctrl/Cmd + 1-4 for quick API testing
    if ((e.ctrlKey || e.metaKey) && e.key >= '1' && e.key <= '4') {
        e.preventDefault();
        const endpoints = ['/api/hello', '/api/status', '/api/time'];
        const index = parseInt(e.key) - 1;

        if (index === 3) {
            testEcho();
        } else if (endpoints[index]) {
            fetchAPI(endpoints[index]);
        }
    }
});

// Display welcome message in console
console.log('%c🚀 Aelys HTTP Server', 'font-size: 20px; font-weight: bold; color: #2563eb;');
console.log('%cKeyboard shortcuts:', 'font-weight: bold; margin-top: 10px;');
console.log('  Ctrl/Cmd + 1: Test /api/hello');
console.log('  Ctrl/Cmd + 2: Test /api/status');
console.log('  Ctrl/Cmd + 3: Test /api/time');
console.log('  Ctrl/Cmd + 4: Test /api/echo');
