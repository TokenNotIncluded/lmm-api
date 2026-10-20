package composition

import (
	"errors"
	"github.com/TokenNotIncluded/api.lmm.best/extensions/internal/modules/toolmarket"
	"net"
	"net/netip"
	"net/url"
	"strconv"
)

// ValidateHostProfile enables HTTP only on a separately selected numeric
// loopback profile. It does not enable HTTP for public provider endpoints.
func ValidateHostProfile(localHTTP bool, listen, origin string) error {
	if !localHTTP {
		if origin == "" {
			return nil
		}
		u, e := url.Parse(origin)
		if e != nil || u.Path != "" || u.RawQuery != "" || u.ForceQuery || u.Fragment != "" || toolmarket.ValidateExternalHTTPURL(origin, false) != nil {
			return errors.New("host public origin must be fixed HTTPS")
		}
		return nil
	}
	u, e := url.Parse(origin)
	if e != nil || u.Scheme != "http" || u.User != nil || u.Path != "" || u.RawQuery != "" || u.ForceQuery || u.Fragment != "" || u.Host != listen {
		return errors.New("local fixture origin must match its loopback listener")
	}
	host, port, e := net.SplitHostPort(listen)
	if e != nil {
		return errors.New("local fixture requires numeric loopback high-port listener")
	}
	ip, e := netip.ParseAddr(host)
	number, pErr := strconv.Atoi(port)
	if e != nil || ip.Zone() != "" || !ip.Unmap().IsLoopback() || pErr != nil || number < 1024 || number > 65535 || strconv.Itoa(number) != port {
		return errors.New("local fixture requires numeric loopback high-port listener")
	}
	return nil
}
func validateLocalCoreOrigin(origin string, cfg Config) error {
	if !cfg.LocalHTTP {
		return errors.New("HTTP Core origin requires the isolated local profile")
	}
	if err := ValidateHostProfile(true, cfg.HostListen, cfg.HostPublicOrigin); err != nil {
		return err
	}
	u, e := url.Parse(origin)
	if e != nil || u.Scheme != "http" || u.Path != "" || u.RawQuery != "" || u.ForceQuery || u.Fragment != "" || u.User != nil {
		return errors.New("invalid isolated Core origin")
	}
	host, port, e := net.SplitHostPort(u.Host)
	if e != nil {
		return errors.New("Core fixture requires numeric loopback high port")
	}
	coreIP, e := netip.ParseAddr(host)
	n, pe := strconv.Atoi(port)
	goURL, _ := url.Parse(cfg.HostPublicOrigin)
	goIP, ge := netip.ParseAddr(goURL.Hostname())
	if e != nil || ge != nil || coreIP.Zone() != "" || !coreIP.Unmap().IsLoopback() || coreIP.Unmap() != goIP.Unmap() || pe != nil || n < 1024 || n > 65535 || strconv.Itoa(n) != port {
		return errors.New("Core fixture origin must share the isolated loopback IP and use an explicit high port")
	}
	return nil
}

// ValidateCorePublicOrigin keeps the separately bound Core browser endpoint on
// the same isolated loopback IP while allowing its own explicit high port.
func ValidateCorePublicOrigin(localHTTP bool, listen, hostOrigin, coreOrigin string) error {
	if localHTTP {
		return validateLocalCoreOrigin(coreOrigin, Config{LocalHTTP: true, HostListen: listen, HostPublicOrigin: hostOrigin})
	}
	u, e := url.Parse(coreOrigin)
	if e != nil || u.Path != "" || u.RawQuery != "" || u.ForceQuery || u.Fragment != "" || toolmarket.ValidateExternalHTTPURL(coreOrigin, false) != nil {
		return errors.New("Core public origin must be fixed HTTPS")
	}
	return nil
}
func validateToolMarketCallbackProfile(cfg Config) error {
	if !cfg.LocalHTTP || cfg.ToolMarketCallbackURL == "" {
		return nil
	}
	if err := ValidateHostProfile(true, cfg.HostListen, cfg.HostPublicOrigin); err != nil {
		return err
	}
	callback, e := url.Parse(cfg.ToolMarketCallbackURL)
	public, _ := url.Parse(cfg.HostPublicOrigin)
	if e != nil || callback.Scheme != "http" || callback.Host != public.Host || callback.User != nil || callback.RawQuery != "" || callback.ForceQuery || callback.Fragment != "" {
		return errors.New("fixture toolmarket callback must use the isolated Go public origin")
	}
	return nil
}
