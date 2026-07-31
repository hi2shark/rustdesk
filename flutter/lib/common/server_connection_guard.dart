bool shouldBlockOutgoingConnectionForServer(bool isUsingPublicServer) =>
    isUsingPublicServer;

void confirmMissingServer(
    void Function() close, void Function() openServerSettings) {
  close();
  openServerSettings();
}
