import { createServer } from 'node:net';
const allocated = new Set();
// Ephemeral loopback ports belong to one combined disposable fixture run.
export async function nextFixturePort() {
  for (let attempt = 0; attempt < 16; attempt++) {
    const server = createServer();
    await new Promise((resolve, reject) => {
      server.once('error', reject);
      server.listen(0, '127.0.0.1', resolve);
    });
    const port = server.address().port;
    await new Promise((resolve, reject) =>
      server.close((error) => (error ? reject(error) : resolve()))
    );
    if (!allocated.has(port)) {
      allocated.add(port);
      return port;
    }
  }
  throw new Error('No distinct loopback fixture port available');
}
