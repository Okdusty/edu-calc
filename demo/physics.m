% Engineering and physics: second-order filter response, signal spectrum, heat diffusion.

% Bode plot of H(s) = w0^2 / (s^2 + 2 zeta w0 s + w0^2) for three damping ratios
w0 = 2*pi*1000;
f = logspace(1, 5, 500);
s = 2i*pi*f;
subplot(2, 2, 1)
hold on
for zeta = [0.1 0.3 0.7]
  H = w0^2 ./ (s.^2 + 2*zeta*w0*s + w0^2);
  semilogx(f, 20*log10(abs(H)))
end
hold off; grid on; legend('\zeta = 0.1', '\zeta = 0.3', '\zeta = 0.7')
title('|H(f)|, resonance at f_0 = 1 kHz'); xlabel('f [Hz]'); ylabel('gain [dB]')

subplot(2, 2, 3)
hold on
for zeta = [0.1 0.3 0.7]
  H = w0^2 ./ (s.^2 + 2*zeta*w0*s + w0^2);
  semilogx(f, angle(H)*180/pi)
end
hold off; grid on
title('phase ∠H(f)'); xlabel('f [Hz]'); ylabel('phase [°]')

% spectrum of a noisy two-tone signal sampled at 1 kHz
randn('seed', 3);
fs = 1000;  n = 1000;  t = (0:n-1)/fs;
x = 0.7*sin(2*pi*50*t) + sin(2*pi*120*t) + 0.8*randn(size(t));
X = abs(fft(x))/n;
P = 2*X(1:n/2+1);  fr = fs*(0:n/2)/n;
subplot(2, 2, 2)
plot(fr, P); grid on
[pk, i] = max(P);
text(fr(i) + 8, pk, sprintf('%g Hz', fr(i)))
text(58, P(51), '50 Hz')
title('single-sided spectrum |X(f)|, fft'); xlabel('f [Hz]'); ylabel('amplitude')

% heat equation u_t = alpha u_xx in a rod, explicit finite differences
L = 1;  nx = 60;  dx = L/(nx - 1);  alpha = 1e-3;
dt = 0.4*dx^2/alpha;  steps = 400;
xs = linspace(0, L, nx);
u = exp(-((xs - 0.3)/0.05).^2) + 0.6*exp(-((xs - 0.7)/0.08).^2);
U = zeros(steps/10, nx);
for k = 1:steps
  u(2:end-1) = u(2:end-1) + alpha*dt/dx^2*(u(3:end) - 2*u(2:end-1) + u(1:end-2));
  if mod(k, 10) == 0
    U(k/10, :) = u;
  end
end
subplot(2, 2, 4)
imagesc([0 L], [0 steps*dt], U); axis xy; colormap('hot'); colorbar   % xy: t runs up
title('heat diffusion u(x, t)'); xlabel('x [m]'); ylabel('t [s]')
