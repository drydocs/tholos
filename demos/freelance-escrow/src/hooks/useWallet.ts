import { useCallback, useEffect, useState } from "react";
import { type WalletState, connectWallet, detectWallet } from "../lib/wallet";

export function useWallet() {
  const [wallet, setWallet] = useState<WalletState>({ status: "disconnected" });
  const [connecting, setConnecting] = useState(false);

  useEffect(() => {
    detectWallet().then(setWallet);
  }, []);

  const connect = useCallback(async () => {
    setConnecting(true);
    try {
      setWallet(await connectWallet());
    } finally {
      setConnecting(false);
    }
  }, []);

  return { wallet, connecting, connect };
}
