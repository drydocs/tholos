import { useWallet } from "../hooks/useWallet";
import { shortenAddress } from "../lib/wallet";

export function WalletButton() {
  const { wallet, connecting, connect } = useWallet();

  if (wallet.status === "unavailable") {
    return (
      <a
        className="wallet-button wallet-button--warn"
        href="https://www.freighter.app/"
        target="_blank"
        rel="noreferrer"
      >
        Install Freighter
      </a>
    );
  }

  if (wallet.status === "connected") {
    return (
      <span className="wallet-button wallet-button--connected">
        {shortenAddress(wallet.address)}
        <span className="wallet-network">{wallet.network}</span>
      </span>
    );
  }

  return (
    <button className="wallet-button" onClick={connect} disabled={connecting}>
      {connecting ? "Connecting..." : "Connect wallet"}
    </button>
  );
}
