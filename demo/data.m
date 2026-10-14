% Data analysis: a year of weather-station readings from CSV (works the same for .tsv/.xlsx/.ods).
T = readtable('demo/weather.csv');      % struct of named columns
D = readmatrix('demo/weather.csv');     % numeric matrix, header skipped
printf('%d days, columns: %s\n', rows(D), strjoin(fieldnames(T)', ', '));

% temperature over the year with a 15-day moving average
subplot(2, 2, 1)
avg = filter(ones(1, 15)/15, 1, T.temperature_C);
plot(T.day, T.temperature_C, '.', T.day(15:end) - 7, avg(15:end), 'LineWidth', 2); grid on
legend('daily', '15-day mean'); title('temperature'); xlabel('day of year'); ylabel('T [°C]')

% distribution with a fitted normal curve
subplot(2, 2, 2)
[n, c] = hist(T.temperature_C, 25);
w = c(2) - c(1);
bar(c, n / (sum(n)*w)); hold on
mu = mean(T.temperature_C);  sd = std(T.temperature_C);
g = linspace(min(c), max(c), 200);
plot(g, exp(-(g - mu).^2/(2*sd^2))/(sd*sqrt(2*pi)), 'LineWidth', 2); hold off; grid on
title(sprintf('N(\\mu = %.1f, \\sigma = %.1f)', mu, sd)); xlabel('T [°C]'); ylabel('density')

% humidity against temperature with a least-squares line
subplot(2, 2, 3)
p = polyfit(T.temperature_C, T.humidity_pct, 1);
r = corrcoef(T.temperature_C, T.humidity_pct);
x = linspace(min(T.temperature_C), max(T.temperature_C), 2);
scatter(T.temperature_C, T.humidity_pct); hold on
plot(x, polyval(p, x), 'LineWidth', 2); hold off; grid on
title(sprintf('h = %.2f T + %.1f,  r = %.2f', p, r(1, 2))); xlabel('T [°C]'); ylabel('humidity [%]')

% correlation between all measured quantities
subplot(2, 2, 4)
matview(corrcoef(D(:, 2:5)))
title('correlation: T, humidity, pressure, wind')
