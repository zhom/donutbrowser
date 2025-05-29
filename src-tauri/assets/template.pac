function FindProxyForURL(url, host) {
  const proxyString = "{{proxy_url}}";

  // Split the proxy string to get the credentials part
  const parts = proxyString.split(" ")[1].split("@");
  if (parts.length > 1) {
    const credentials = parts[0];
    const encodedCredentials = encodeURIComponent(credentials);
    // Replace the original credentials with encoded ones
    return proxyString.replace(credentials, encodedCredentials);
  }

  return proxyString;
}
