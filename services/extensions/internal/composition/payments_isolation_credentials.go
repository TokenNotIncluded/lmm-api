package composition

// These are the same frozen bytes used by the already composed module clients.
// Keep startup-only references until the payment lab constructor checks them;
// do not reopen credential paths, which could rotate between checks and use.
func (r *Runtime) retainPaymentIsolationCredentials(selected []string, credentials ...[]byte) {
	for _, name := range selected {
		if name == "payments" {
			r.paymentIsolationCredentials = append(r.paymentIsolationCredentials, credentials...)
			return
		}
	}
}
